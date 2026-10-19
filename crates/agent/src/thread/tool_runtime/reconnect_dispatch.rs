//! Accepted calls sharing one reconnect receipt launch in their original order.
//! Later direct calls after connection retain their existing admission path.
use super::super::{
    bound_call::BoundToolCall,
    connection_wait::{ConnectionWaitError, WaitTarget},
    session_driver::OperationGuard,
};
use gpui_kit::{AnyWindowHandle, App, EntityId, WeakEntity};
use nocterm_ai::{BridgeCall, session::Ticket};
use nocterm_workspace::{ExpectedSession, Workspace};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::thread) struct DispatchScope {
    receipt: ExpectedSession,
    ticket: Ticket,
    registration: Option<u64>,
    workspace: EntityId,
    window: AnyWindowHandle,
}
impl DispatchScope {
    pub(in crate::thread) fn new(target: &WaitTarget) -> Self {
        Self {
            receipt: target.expected(),
            ticket: target.ticket(),
            registration: target.registration(),
            workspace: target.workspace().entity_id(),
            window: target.window(),
        }
    }
    pub(in crate::thread) fn ticket(&self) -> Ticket {
        self.ticket
    }
    pub(in crate::thread) fn receipt(&self) -> ExpectedSession {
        self.receipt
    }
    pub(in crate::thread) fn is_current(
        &self,
        workspace: &WeakEntity<Workspace>,
        window: Option<AnyWindowHandle>,
        session_current: bool,
        registration: Option<u64>,
        cx: &App,
    ) -> bool {
        self.workspace == workspace.entity_id()
            && workspace.upgrade().is_some()
            && window == Some(self.window)
            && cx.windows().contains(&self.window)
            && session_current
            && registration == self.registration
    }
}

#[derive(Clone)]
pub(in crate::thread) struct DispatchTicket(Arc<()>);
struct PendingDispatch {
    identity: DispatchTicket,
    scope: DispatchScope,
    call: BoundToolCall,
    readiness: Option<Result<ExpectedSession, ConnectionWaitError>>,
    guard: OperationGuard,
}
pub(in crate::thread) struct DispatchEffect {
    scope: DispatchScope,
    call: BoundToolCall,
    result: Result<ExpectedSession, ConnectionWaitError>,
    guard: OperationGuard,
}
impl DispatchEffect {
    pub(in crate::thread) fn into_parts(
        self,
    ) -> (
        DispatchScope,
        BoundToolCall,
        Result<ExpectedSession, ConnectionWaitError>,
        OperationGuard,
    ) {
        (self.scope, self.call, self.result, self.guard)
    }
}

#[derive(Default)]
pub(super) struct ReconnectDispatch {
    pending: Vec<PendingDispatch>,
}
impl ReconnectDispatch {
    pub(super) fn admit(
        &mut self,
        scope: DispatchScope,
        call: BoundToolCall,
        guard: OperationGuard,
    ) -> DispatchTicket {
        let identity = DispatchTicket(Arc::new(()));
        self.pending.push(PendingDispatch {
            identity: identity.clone(),
            scope,
            call,
            readiness: None,
            guard,
        });
        identity
    }
    pub(super) fn ready(
        &mut self,
        ticket: &DispatchTicket,
        result: Result<ExpectedSession, ConnectionWaitError>,
    ) -> Option<DispatchScope> {
        let pending = self
            .pending
            .iter_mut()
            .find(|pending| Arc::ptr_eq(&pending.identity.0, &ticket.0))?;
        if pending.readiness.is_some() {
            return None;
        }
        pending.readiness = Some(result);
        Some(pending.scope)
    }
    pub(super) fn take_ready(&mut self, scope: DispatchScope) -> Option<DispatchEffect> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.scope == scope)?;
        let result = self.pending[index].readiness.take()?;
        let pending = self.pending.remove(index);
        Some(DispatchEffect {
            scope: pending.scope,
            call: pending.call,
            result,
            guard: pending.guard,
        })
    }
    pub(super) fn cancel(&mut self) -> Vec<BridgeCall> {
        self.pending
            .drain(..)
            .map(|pending| pending.call.into_call())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::session_driver::SessionDriver;
    use futures::channel::oneshot;
    use nocterm_ai::{SendInput, TerminalCall};

    fn scope(window: AnyWindowHandle) -> DispatchScope {
        DispatchScope {
            receipt: ExpectedSession::new(EntityId::from(1), 2),
            ticket: nocterm_ai::session::SessionState::default().ticket(),
            registration: Some(3),
            workspace: EntityId::from(4),
            window,
        }
    }
    fn call(text: &str) -> BoundToolCall {
        let (respond, _) = oneshot::channel();
        BoundToolCall::new(
            BridgeCall {
                call: TerminalCall::SendInput(SendInput {
                    terminal_id: "terminal".into(),
                    text: text.into(),
                    press_enter: true,
                }),
                arguments: None,
                display_token: None,
                registration_id: 3,
                respond,
            },
            None,
        )
    }
    fn text(call: &BoundToolCall) -> &str {
        match &call.call().call {
            TerminalCall::SendInput(input) => &input.text,
            _ => panic!("expected terminal input"),
        }
    }

    #[gpui_kit::test]
    fn later_readiness_waits_for_the_original_head_and_keeps_work_busy(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let scope = scope(
            cx.add_empty_window()
                .update(|window, _| window.window_handle()),
        );
        let session = SessionDriver::default();
        let mut dispatch = ReconnectDispatch::default();
        let first = dispatch.admit(scope, call("first"), session.hold_operation());
        let second = dispatch.admit(scope, call("second"), session.hold_operation());
        assert_eq!(dispatch.ready(&second, Ok(scope.receipt())), Some(scope));
        assert!(dispatch.take_ready(scope).is_none());
        assert!(session.busy());
        assert_eq!(dispatch.ready(&first, Ok(scope.receipt())), Some(scope));
        let (_, first_call, result, guard) = dispatch.take_ready(scope).unwrap().into_parts();
        assert_eq!(text(&first_call), "first");
        assert_eq!(result.unwrap(), scope.receipt());
        assert!(session.busy());
        drop(guard);
        let (_, second_call, _, guard) = dispatch.take_ready(scope).unwrap().into_parts();
        assert_eq!(text(&second_call), "second");
        assert!(session.busy());
        drop(guard);
        assert!(!session.busy());
        assert!(dispatch.take_ready(scope).is_none());
    }

    #[gpui_kit::test]
    fn a_failed_head_advances_without_replacing_its_recorded_result(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let scope = scope(
            cx.add_empty_window()
                .update(|window, _| window.window_handle()),
        );
        let session = SessionDriver::default();
        let mut dispatch = ReconnectDispatch::default();
        let first = dispatch.admit(scope, call("first"), session.hold_operation());
        let second = dispatch.admit(scope, call("second"), session.hold_operation());
        dispatch.ready(&second, Ok(scope.receipt()));
        dispatch.ready(&first, Err(ConnectionWaitError::Stopped));
        assert!(dispatch.ready(&first, Ok(scope.receipt())).is_none());
        let (_, first_call, result, guard) = dispatch.take_ready(scope).unwrap().into_parts();
        assert_eq!(text(&first_call), "first");
        assert!(matches!(result, Err(ConnectionWaitError::Stopped)));
        drop(guard);
        let (_, second_call, result, guard) = dispatch.take_ready(scope).unwrap().into_parts();
        assert_eq!(text(&second_call), "second");
        assert!(result.is_ok());
        drop(guard);
        assert!(!session.busy());
    }

    #[gpui_kit::test]
    fn cancellation_releases_waiters_and_old_tickets_cannot_complete_replacements(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let scope = scope(
            cx.add_empty_window()
                .update(|window, _| window.window_handle()),
        );
        let session = SessionDriver::default();
        let mut dispatch = ReconnectDispatch::default();
        let first = dispatch.admit(scope, call("first"), session.hold_operation());
        let second = dispatch.admit(scope, call("second"), session.hold_operation());
        dispatch.ready(&second, Ok(scope.receipt()));
        let cancelled = dispatch.cancel();
        assert_eq!(cancelled.len(), 2);
        assert!(!session.busy());
        assert!(dispatch.cancel().is_empty());
        let replacement = dispatch.admit(scope, call("replacement"), session.hold_operation());
        assert!(dispatch.ready(&first, Ok(scope.receipt())).is_none());
        assert!(dispatch.ready(&second, Ok(scope.receipt())).is_none());
        assert!(dispatch.take_ready(scope).is_none());
        dispatch.ready(&replacement, Ok(scope.receipt()));
        let (_, call, _, guard) = dispatch.take_ready(scope).unwrap().into_parts();
        assert_eq!(text(&call), "replacement");
        drop(guard);
        assert!(!session.busy());
    }

    #[gpui_kit::test]
    fn another_registration_does_not_wait_for_an_unready_scope(cx: &mut gpui_kit::TestAppContext) {
        let scope = scope(
            cx.add_empty_window()
                .update(|window, _| window.window_handle()),
        );
        let other = DispatchScope {
            registration: Some(9),
            ..scope
        };
        let session = SessionDriver::default();
        let mut dispatch = ReconnectDispatch::default();
        dispatch.admit(scope, call("waiting"), session.hold_operation());
        let ready = dispatch.admit(other, call("independent"), session.hold_operation());
        dispatch.ready(&ready, Ok(other.receipt()));
        let (_, call, _, guard) = dispatch.take_ready(other).unwrap().into_parts();
        assert_eq!(text(&call), "independent");
        drop(guard);
        assert!(session.busy());
        assert!(dispatch.take_ready(scope).is_none());
        dispatch.cancel();
        assert!(!session.busy());
    }
}
