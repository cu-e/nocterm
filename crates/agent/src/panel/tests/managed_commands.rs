//! Session operations of the fixture agent; the shared script owns close and shutdown.
use super::*;
use nocterm_ai::testing::{ScriptedOperation as Operation, SessionScript};
use nocterm_ai::{
    connection::ManagedPreparedPrompt,
    history::{
        physical::DispatchPayload,
        source::{ManagedCommandError, ManagedOpened, ManagedSourceFuture, SourceLifetime},
    },
    model::{
        json::deferred::NativeAllocationError,
        native::{NativeCopies, NativeOwned},
        session::RestoreSession,
    },
};
use std::future::Future;
mod replies;
/// One prompt as the agent received it.
pub(super) struct PromptRecord(NativeOwned<PromptRequest>);
impl std::ops::Deref for PromptRecord {
    type Target = PromptRequest;
    fn deref(&self) -> &PromptRequest {
        self.0.get()
    }
}
pub(super) struct SessionFixture {
    commands: Arc<Commands>,
}
impl SessionFixture {
    pub(super) fn new(commands: Arc<Commands>) -> Self {
        Self { commands }
    }
}
fn work<T: Send + 'static>(
    lifetime: &SourceLifetime,
    future: impl Future<Output = Result<T, ManagedCommandError>> + Send + 'static,
) -> Operation<T> {
    ManagedSourceFuture::new(lifetime.clone(), future)
}
impl SessionScript for SessionFixture {
    fn new_session(
        &self,
        request: &SessionOpen,
        lifetime: &SourceLifetime,
    ) -> Operation<ManagedOpened> {
        self.commands
            .new_dirs
            .lock()
            .unwrap()
            .push(request.working_directory.clone());
        self.commands
            .servers
            .lock()
            .unwrap()
            .push(request.endpoints.clone());
        let gate = self.commands.session_gate.lock().unwrap().take();
        let id = self.commands.sessions.fetch_add(1, Ordering::SeqCst);
        let commands = self.commands.clone();
        let context = lifetime.context_ref().clone();
        let prepared = replies::OpenedReply::new(&context)?;
        work(lifetime, async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            if commands.auth_required.load(Ordering::SeqCst) {
                return Err(RestoreFailure::Authentication("Sign in to continue")
                    .managed(&context)
                    .map_err(ManagedCommandError::Admission)?);
            }
            prepared.finish(&commands, &format!("s{id}"))
        })
    }
    fn restore_session(
        &self,
        request: &RestoreSession,
        lifetime: &SourceLifetime,
    ) -> Operation<ManagedOpened> {
        self.commands
            .restore_dirs
            .lock()
            .unwrap()
            .push(request.working_directory.clone());
        self.commands
            .restores
            .lock()
            .unwrap()
            .push((request.session.clone(), request.fork));
        let context = lifetime.context_ref().clone();
        let scripted = self
            .commands
            .restore_errors
            .lock()
            .unwrap()
            .pop_front()
            .map(|error| error.managed(&context))
            .transpose()?;
        let session = if request.fork {
            format!("fork-of-{}", request.session)
        } else {
            request.session.to_string()
        };
        let prepared = replies::OpenedReply::new(&context)?;
        let commands = self.commands.clone();
        work(lifetime, async move {
            if let Some(error) = scripted {
                Err(error)
            } else {
                prepared.finish(&commands, &session)
            }
        })
    }
    fn prepare_prompt(
        &self,
        payload: &DispatchPayload,
        lifetime: &SourceLifetime,
    ) -> Result<ManagedPreparedPrompt, ManagedCommandError> {
        let context = lifetime.context_ref();
        if self.commands.prompt_refused.load(Ordering::SeqCst) {
            let refused = context
                .prepare_error::<scripted_failure::FixtureIo>(0)
                .map_err(ManagedCommandError::Admission)?;
            return Err(ManagedCommandError::Native(
                refused.capture(scripted_failure::FixtureIo("staging refused")),
            ));
        }
        let copy_failure = context
            .prepare_error::<nocterm_ai::model::json::deferred::JsonDecodeError>(0)
            .map_err(ManagedCommandError::Admission)?;
        let request = NativeCopies::new(context.ledger())
            .prompt(payload)
            .map_err(|error| ManagedCommandError::Native(copy_failure.capture(error)))?;
        let auth = RestoreFailure::Authentication("Sign in to continue")
            .managed(context)
            .map_err(ManagedCommandError::Admission)?;
        let closed = context
            .prepare_error::<scripted_failure::FixtureIo>(0)
            .map_err(ManagedCommandError::Admission)?;
        let result_copy = context
            .prepare_error::<nocterm_ai::model::json::deferred::JsonDecodeError>(0)
            .map_err(ManagedCommandError::Admission)?;
        let commands = self.commands.clone();
        let context = context.clone();
        let work = work(lifetime, async move {
            commands.prompts.lock().unwrap().push(PromptRecord(request));
            if commands.auth_required.load(Ordering::SeqCst) {
                return Err(auth);
            }
            let (tx, rx) = oneshot::channel();
            *commands.pending.lock().unwrap() = Some(tx);
            let outcome = rx.await.map_err(|_| {
                ManagedCommandError::Native(closed.capture(scripted_failure::FixtureIo("closed")))
            })?;
            NativeCopies::new(context.ledger())
                .outcome(&outcome)
                .map_err(|error| ManagedCommandError::Native(result_copy.capture(error)))
        })
        .map_err(ManagedCommandError::Admission)?;
        Ok(ManagedPreparedPrompt::new(work))
    }
    fn cancel(&self, _: &SessionId) -> Result<(), ManagedCommandError> {
        self.commands.cancels.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn set_mode(&self, _: &ModeSelection, lifetime: &SourceLifetime) -> Operation<()> {
        work(lifetime, async { Ok(()) })
    }
    fn set_config_option(
        &self,
        request: &ConfigurationSelection,
        lifetime: &SourceLifetime,
    ) -> Operation<NativeOwned<Vec<ConfigOption>>> {
        let context = lifetime.context_ref().clone();
        let failure =
            context.prepare_error::<nocterm_ai::model::json::deferred::JsonDecodeError>(0)?;
        let output = self.commands.config.choose_native(request, &context);
        work(lifetime, async move {
            output.map_err(|error| ManagedCommandError::Native(failure.capture(error)))
        })
    }
    fn authenticate(&self, _: &AuthMethodId, lifetime: &SourceLifetime) -> Operation<()> {
        self.commands.authentications.fetch_add(1, Ordering::SeqCst);
        let failed = self.commands.authentication_failure.load(Ordering::SeqCst);
        if !failed {
            self.commands.auth_required.store(false, Ordering::SeqCst);
        }
        let failure = lifetime
            .context_ref()
            .prepare_error::<scripted_failure::FixtureIo>(0)?;
        work(lifetime, async move {
            if failed {
                Err(ManagedCommandError::Native(
                    failure.capture(scripted_failure::FixtureIo("Sign-in failed")),
                ))
            } else {
                Ok(())
            }
        })
    }
}
