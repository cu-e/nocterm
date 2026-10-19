//! Original responder, cancellation and captured offer remain one actual task.
use super::{budget::SourceBudget, driver::DriverError, outgoing, permission::SelectionAuthority};
use agent_client_protocol::managed::{Cancelled, Dispatch, Responder};
use nocterm_ai::model::auth::{PermissionAnswer, PermissionOutcome};
use serde::Serialize;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
#[derive(Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum Outcome<'a> {
    Cancelled,
    Selected {
        #[serde(rename = "optionId")]
        option: &'a str,
    },
}
#[derive(Serialize)]
struct Answer<'a> {
    outcome: Outcome<'a>,
}
pub(super) struct PermissionTask {
    responder: Option<Responder<SourceBudget>>,
    cancelled: Cancelled<SourceBudget>,
    answer: PermissionAnswer,
    authority: SelectionAuthority,
    _dispatch: Dispatch<SourceBudget>,
    budget: SourceBudget,
}
impl PermissionTask {
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn reject_overflow(mut self) -> Result<(), DriverError> {
        if let Some(responder) = self.responder.take() {
            let response = outgoing::fault::<()>(
                &self.budget,
                responder.id(),
                -32603,
                "Internal error",
                None,
            )?;
            responder
                .respond(response)
                .map_err(|(error, _, _)| DriverError::Routing(error))?;
        }
        Ok(())
    }
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn new(
        responder: Responder<SourceBudget>,
        answer: PermissionAnswer,
        authority: SelectionAuthority,
        dispatch: Dispatch<SourceBudget>,
        budget: SourceBudget,
    ) -> Result<Self, DriverError> {
        let cancelled = responder.cancellation()?.cancelled()?;
        Ok(Self {
            responder: Some(responder),
            cancelled,
            answer,
            authority,
            _dispatch: dispatch,
            budget,
        })
    }
}
impl Future for PermissionTask {
    type Output = Result<(), DriverError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let outcome = if Pin::new(&mut this.cancelled).poll(cx).is_ready() {
            PermissionOutcome::Cancelled
        } else {
            match Pin::new(&mut this.answer).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(value)) => value,
                Poll::Ready(Err(_)) => PermissionOutcome::Cancelled,
            }
        };
        let Some(responder) = this.responder.take() else {
            return Poll::Pending;
        };
        let selected = this.authority.selection(&outcome);
        let answer = Answer {
            outcome: match selected {
                Some(option) => Outcome::Selected { option },
                None => Outcome::Cancelled,
            },
        };
        let response = outgoing::result(&this.budget, responder.id(), &answer)?;
        responder
            .respond(response)
            .map_err(|(error, _, _)| DriverError::Routing(error))?;
        Poll::Ready(Ok(()))
    }
}
