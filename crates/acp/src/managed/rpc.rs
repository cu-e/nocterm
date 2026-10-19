//! Paid unpolled RPC work keeps ordered responses through decode and certification.
use super::{
    budget::SourceBudget,
    diagnostics::{self, Diagnostics},
    error::DecodeError,
    models::Models,
    state::State,
    value::Value,
};
use agent_client_protocol::managed::{
    Connection, Envelope, ManagedError, MessageKind, Request, Shared,
};
use nocterm_ai::{
    history::source::{
        ManagedCommandError, ManagedOpened, ManagedSourceFuture, SourceErrorSlot,
        SourceSemanticRefusal,
    },
    model::{
        json::deferred::{JsonDecodeError, NativeAllocationError},
        session::SessionOpened,
    },
};
use std::sync::Mutex;
#[derive(Debug, thiserror::Error)]
pub(super) enum Cause {
    #[error(transparent)]
    Decode(#[from] DecodeError<JsonDecodeError>),
    #[error(transparent)]
    Routing(#[from] ManagedError<JsonDecodeError>),
    #[error(transparent)]
    Diagnostic(#[from] diagnostics::Error),
    #[error(transparent)]
    Opened(#[from] SourceSemanticRefusal<SessionOpened>),
    #[error("{0}")]
    Timeout(&'static str),
    #[error("Unsupported ACP protocol version")]
    ProtocolVersion,
    #[error(transparent)]
    Info(#[from] SourceSemanticRefusal<nocterm_ai::AgentInfo>),
    #[error(transparent)]
    Cleanup(#[from] crate::process::CleanupError),
}
pub(super) fn failure(error: Cause, ticket: SourceErrorSlot<Cause>) -> ManagedCommandError {
    let admission = match &error {
        Cause::Decode(error) => super::native_admission::decode(error),
        Cause::Routing(error) => super::native_admission::sdk(error),
        Cause::Diagnostic(error) => super::native_admission::diagnostic(error),
        Cause::Opened(error) => super::native_admission::semantic(error),
        Cause::Info(error) => super::native_admission::semantic(error),
        _ => None,
    };
    match admission {
        Some(error) => ManagedCommandError::Admission(error),
        None => ManagedCommandError::Native(ticket.capture(error)),
    }
}
#[derive(Clone)]
pub(super) struct Rpc {
    connection: Connection<SourceBudget>,
    budget: SourceBudget,
    diagnostics: Shared<Diagnostics, SourceBudget>,
    models: Shared<Mutex<Models>, SourceBudget>,
    state: State,
}
impl Rpc {
    pub(super) fn new(
        connection: Connection<SourceBudget>,
        budget: SourceBudget,
        diagnostics: Shared<Diagnostics, SourceBudget>,
        models: Shared<Mutex<Models>, SourceBudget>,
        state: State,
    ) -> Self {
        Self {
            connection,
            budget,
            diagnostics,
            models,
            state,
        }
    }
    pub(super) fn budget(&self) -> &SourceBudget {
        &self.budget
    }
    pub(super) fn state(&self) -> &State {
        &self.state
    }
    pub(super) fn models(&self) -> &Shared<Mutex<Models>, SourceBudget> {
        &self.models
    }
    pub(super) fn id(
        &self,
    ) -> Result<
        agent_client_protocol::managed::MessageId<SourceBudget>,
        ManagedError<JsonDecodeError>,
    > {
        self.connection.next_id()
    }
    pub(super) fn notify(
        &self,
        envelope: Envelope<SourceBudget>,
    ) -> Result<(), ManagedError<JsonDecodeError>> {
        self.connection.notify(envelope).map_err(|(error, _)| error)
    }
    pub(super) fn ticket(&self) -> Result<SourceErrorSlot<Cause>, NativeAllocationError> {
        self.budget.context().prepare_error(0)
    }
    pub(super) fn operation<T, F>(
        &self,
        envelope: Result<Envelope<SourceBudget>, Cause>,
        restoring: bool,
        decode: F,
    ) -> Result<ManagedSourceFuture<Result<T, ManagedCommandError>>, NativeAllocationError>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(Value<'a>, &mut Models) -> Result<T, Cause> + Send + 'static,
    {
        self.operation_with_timeout(envelope, restoring, None, decode)
    }
    pub(super) fn operation_with_timeout<T, F>(
        &self,
        envelope: Result<Envelope<SourceBudget>, Cause>,
        restoring: bool,
        timeout: Option<std::time::Duration>,
        decode: F,
    ) -> Result<ManagedSourceFuture<Result<T, ManagedCommandError>>, NativeAllocationError>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(Value<'a>, &mut Models) -> Result<T, Cause> + Send + 'static,
    {
        let lifetime = self.state.lifetime()?;
        let ticket = self.ticket()?;
        let rpc = self.clone();
        ManagedSourceFuture::new(lifetime, async move {
            match rpc.execute(envelope, restoring, timeout, decode).await {
                Ok(value) => Ok(value),
                Err(Failure::Remote(error)) => Err(error),
                Err(Failure::Native(error)) => Err(failure(error, ticket)),
            }
        })
    }
    async fn execute<T, F>(
        &self,
        envelope: Result<Envelope<SourceBudget>, Cause>,
        restoring: bool,
        timeout: Option<std::time::Duration>,
        decode: F,
    ) -> Result<T, Failure>
    where
        F: for<'a> FnOnce(Value<'a>, &mut Models) -> Result<T, Cause>,
    {
        let timeout_message = if envelope
            .as_ref()
            .is_ok_and(|value| value.method() == Some("initialize"))
        {
            "Agent initialization timed out"
        } else {
            "Agent operation timed out"
        };
        let request = self
            .connection
            .request(envelope?)
            .map_err(|(error, _)| Cause::Routing(error))?
            .ordered();
        self.execute_request(request, restoring, timeout, decode, timeout_message)
            .await
    }
    async fn execute_request<T, F>(
        &self,
        request: Request<SourceBudget>,
        restoring: bool,
        timeout: Option<std::time::Duration>,
        decode: F,
        timeout_message: &'static str,
    ) -> Result<T, Failure>
    where
        F: for<'a> FnOnce(Value<'a>, &mut Models) -> Result<T, Cause>,
    {
        let response = if let Some(timeout) = timeout {
            let deadline = async_io::Timer::after(timeout);
            futures::pin_mut!(request, deadline);
            match futures::future::select(request, deadline).await {
                futures::future::Either::Left((response, _)) => response.map_err(Cause::Routing)?,
                futures::future::Either::Right(_) => {
                    return Err(Cause::Timeout(timeout_message).into());
                }
            }
        } else {
            request.await.map_err(Cause::Routing)?
        };
        if response.envelope().kind() == MessageKind::Error {
            return Err(Failure::Remote(
                self.diagnostics
                    .get()
                    .remote(response.envelope(), restoring)
                    .map_err(Cause::Diagnostic)?,
            ));
        }
        let span = response
            .envelope()
            .payload()
            .supplied()
            .ok_or(DecodeError::Field("RPC result"))
            .map_err(Cause::Decode)?;
        let value = Value::from_span(span, &self.budget).map_err(Cause::Decode)?;
        let mut models = self
            .models
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // The real Response remains alive until independent cache/delivery owners
        // and their semantic certificate have all been admitted successfully.
        let decoded = decode(value, &mut models)?;
        drop(models);
        drop(response);
        Ok(decoded)
    }
    pub(super) fn prepare_prompt(
        &self,
        payload: &nocterm_ai::history::physical::DispatchPayload,
    ) -> Result<nocterm_ai::connection::ManagedPreparedPrompt, ManagedCommandError> {
        let lifetime = self
            .state
            .lifetime()
            .map_err(ManagedCommandError::Admission)?;
        let ticket = self.ticket().map_err(ManagedCommandError::Admission)?;
        let prepared = (|| {
            let id = self.id()?;
            let envelope = super::outgoing::prompt(&self.budget, &id, payload)?;
            Ok::<_, Cause>(
                self.connection
                    .prepare_request(envelope)
                    .map_err(|(error, _)| Cause::Routing(error))?
                    .ordered(),
            )
        })();
        let request = match prepared {
            Ok(request) => request,
            Err(error) => return Err(failure(error, ticket)),
        };
        let rpc = self.clone();
        let work = ManagedSourceFuture::new(lifetime, async move {
            match rpc
                .execute_request(
                    request,
                    false,
                    None,
                    |value, _| Ok(super::results::outcome(value)?),
                    "Agent operation timed out",
                )
                .await
            {
                Ok(outcome) => Ok(outcome),
                Err(Failure::Remote(error)) => Err(error),
                Err(Failure::Native(error)) => Err(failure(error, ticket)),
            }
        })
        .map_err(ManagedCommandError::Admission)?;
        Ok(nocterm_ai::connection::ManagedPreparedPrompt::new(work))
    }
    pub(super) fn unsupported<T: Send + 'static>(
        &self,
        fork: bool,
    ) -> Result<ManagedSourceFuture<Result<T, ManagedCommandError>>, NativeAllocationError> {
        ManagedSourceFuture::new(self.state.lifetime()?, async move {
            Err(ManagedCommandError::LocalRestoreUnsupported { fork })
        })
    }
    pub(super) fn opened(
        &self,
        envelope: Result<Envelope<SourceBudget>, Cause>,
        restoring: bool,
        restored: Option<agent_client_protocol::managed::Source<SourceBudget>>,
    ) -> Result<
        ManagedSourceFuture<Result<ManagedOpened, ManagedCommandError>>,
        NativeAllocationError,
    > {
        let context = self.budget.context().clone();
        // A fork restores an existing session under a new identity.
        self.operation(envelope, restoring, move |value, models| {
            let prepared = super::results::prepared_opened(
                value,
                restored.as_ref().map(|source| source.text()),
            )?;
            let (opened, selector) = prepared.into_parts();
            let opened = context.certify_opened(opened)?;
            super::results::install_opened(models, opened.opened(), selector.as_ref())?;
            Ok(opened)
        })
    }
}
enum Failure {
    Remote(ManagedCommandError),
    Native(Cause),
}
impl From<Cause> for Failure {
    fn from(cause: Cause) -> Self {
        Self::Native(cause)
    }
}
