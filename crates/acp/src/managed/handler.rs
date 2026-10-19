//! Real incoming notifications and permission tasks share original driver order.
use super::{
    authentication,
    budget::SourceBudget,
    channel::Sender,
    driver::{DriverError, Handler},
    models::Models,
    notice,
    permission::SelectionAuthority,
    permission_task::PermissionTask,
    state::State,
};
use agent_client_protocol::managed::{
    Buffer, Dispatch, Driver, MessageId, MessageKind, Shared, Source,
};
use nocterm_ai::{connection::ManagedAgentEvent, model::auth::PermissionResponder};
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
};
pub(super) struct Incoming {
    events: Sender<ManagedAgentEvent>,
    models: Shared<Mutex<Models>, SourceBudget>,
    permissions: Buffer<PermissionTask, SourceBudget>,
    state: State,
    budget: SourceBudget,
}
impl Incoming {
    pub(super) fn new(
        events: Sender<ManagedAgentEvent>,
        models: Shared<Mutex<Models>, SourceBudget>,
        state: State,
        budget: SourceBudget,
    ) -> Self {
        Self {
            events,
            models,
            permissions: Buffer::new(budget.clone()),
            state,
            budget,
        }
    }
    fn permission(&mut self, mut dispatch: Dispatch<SourceBudget>) -> Result<(), DriverError> {
        let span = dispatch.envelope().payload().supplied();
        let schema = super::permission_schema::diagnostic(span, &self.budget)?;
        let Some(responder) = dispatch.take_responder() else {
            return Ok(());
        };
        if let Some(error) = schema {
            let response = super::permission_schema::response(
                &self.budget,
                responder.id(),
                dispatch.envelope().payload().supplied(),
                &error,
            )?;
            responder
                .respond(response)
                .map_err(|(error, _, _)| DriverError::Routing(error))?;
            return Ok(());
        }
        let span = dispatch
            .envelope()
            .payload()
            .supplied()
            .ok_or(super::error::DecodeError::Field("permission params"))?;
        let request = match authentication::permission(span, &self.budget) {
            Ok(request) => request,
            Err(error) if super::native_admission::processing(&error) => {
                let text = super::diagnostic_text::format(format_args!("{error}"), &self.budget)?;
                let response = super::outgoing::fault(
                    &self.budget,
                    responder.id(),
                    -32603,
                    "Internal error",
                    Some(&text.text()),
                )?;
                responder
                    .respond(response)
                    .map_err(|(error, _, _)| DriverError::Routing(error))?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let authority = SelectionAuthority::capture(&request, self.budget.clone())?;
        let request = self
            .budget
            .context()
            .certify_permission(request)
            .map_err(DriverError::Permission)?;
        let (respond, answer) = PermissionResponder::managed_channel(self.budget.context())
            .map_err(super::error::DecodeError::Native)?;
        let task =
            PermissionTask::new(responder, answer, authority, dispatch, self.budget.clone())?;
        // Allocate its real task position before publishing the one UI responder.
        self.permissions.reserve(1)?;
        if self
            .events
            .try_send(ManagedAgentEvent::Permission { request, respond })
            .is_err()
        {
            return task.reject_overflow();
        }
        self.permissions
            .push(task)
            .map_err(|(error, _)| DriverError::Routing(error))?;
        Ok(())
    }
}
impl Handler for Incoming {
    fn dispatch(
        &mut self,
        dispatch: Dispatch<SourceBudget>,
        driver: &Driver<SourceBudget>,
    ) -> Result<(), DriverError> {
        let result = (|| match (dispatch.envelope().kind(), dispatch.envelope().method()) {
            (MessageKind::Request, Some("session/request_permission")) => self.permission(dispatch),
            (MessageKind::Notification, Some("session/update")) => {
                let Some(span) = dispatch.envelope().payload().supplied() else {
                    return Ok(());
                };
                let mut models = self
                    .models
                    .get()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(notice) =
                    notice::notice_with_models(span, &self.budget, Some(&mut models))?
                {
                    let notice = self
                        .budget
                        .context()
                        .certify_notice(notice)
                        .map_err(DriverError::Notice)?;
                    let _ = self.events.try_send(ManagedAgentEvent::Session(notice));
                }
                Ok(())
            }
            (MessageKind::Notification, Some("$/cancel_request")) => {
                let span = dispatch
                    .envelope()
                    .payload()
                    .supplied()
                    .ok_or(super::error::DecodeError::Field("cancellation params"))?;
                let root = super::value::Value::from_span(span, &self.budget)?;
                let [id] = root.fields(["requestId"])?;
                let id = super::value::Value::required(id, "requestId")?;
                let id = if id.raw() == "null" {
                    MessageId::Null
                } else if id.raw().starts_with('"') {
                    MessageId::String(
                        Source::copy(id.text()?.as_str(), self.budget.clone())?.full(),
                    )
                } else {
                    MessageId::Number(
                        id.raw()
                            .parse()
                            .map_err(|_| super::error::DecodeError::Field("requestId"))?,
                    )
                };
                driver.cancel_incoming(&id);
                Ok(())
            }
            _ => {
                dispatch.unhandled();
                Ok(())
            }
        })();
        match result {
            Err(DriverError::Decode(error)) if super::native_admission::processing(&error) => {
                tracing::warn!(%error,"Ignoring native notification processing error");
                Ok(())
            }
            result => result,
        }
    }
    fn poll_work(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), DriverError>> {
        let mut index = 0;
        while index < self.permissions.len() {
            match Pin::new(&mut self.permissions.as_mut_slice()[index]).poll(cx) {
                Poll::Pending => index += 1,
                Poll::Ready(result) => {
                    drop(self.permissions.remove(index));
                    result?;
                }
            }
        }
        Poll::Pending
    }
    fn closed(&mut self) {
        self.permissions.clear();
        self.state.close();
    }
}
