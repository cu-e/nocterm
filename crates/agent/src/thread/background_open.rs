//! A background opening owns its original workspace receipt independently of chat lifetime.
use super::{
    AgentThread,
    connection_wait::ConnectionWaitError,
    connection_wait::{ConnectionWaitOperation, WaitTarget},
};
use crate::seat::{Seat, current_session};
use gpui_kit::{AnyWindowHandle, AppContext as _, AsyncApp, WeakEntity};
use nocterm_workspace_ports::ExpectedSession;

pub(super) struct BackgroundOpen {
    seat: Seat,
    window: AnyWindowHandle,
    profile: String,
    ticket: nocterm_ai::session::Ticket,
    registration: Option<u64>,
}
impl BackgroundOpen {
    pub(super) fn seat(&self) -> &Seat {
        &self.seat
    }
    pub(super) fn ticket(&self) -> nocterm_ai::session::Ticket {
        self.ticket
    }
    pub(super) fn registration(&self) -> Option<u64> {
        self.registration
    }
    pub(super) fn profile(&self) -> &str {
        &self.profile
    }

    pub(super) fn new(
        seat: Seat,
        window: AnyWindowHandle,
        profile: String,
        ticket: nocterm_ai::session::Ticket,
        registration: Option<u64>,
    ) -> Self {
        Self {
            seat,
            window,
            profile,
            ticket,
            registration,
        }
    }

    pub(super) async fn open(
        &self,
        thread: &WeakEntity<AgentThread>,
        cx: &mut AsyncApp,
    ) -> Result<serde_json::Value, ConnectionWaitError> {
        let opened = self.create(thread, cx)?;
        let registered = thread
            .update(cx, |thread, cx| {
                thread.accept_background_open(self, opened.item, cx)
            })
            .map_err(|_| ConnectionWaitError::ChatClosed);
        let result = match (registered, opened.expected) {
            (Ok(Ok(())), Some(expected)) => {
                ConnectionWaitOperation::new(WaitTarget::new(
                    self.seat.clone(),
                    self.window,
                    expected,
                    self.ticket,
                    self.registration,
                ))
                .run(thread, cx)
                .await
            }
            (Ok(Err(error)) | Err(error), _) => Err(error),
            (Ok(Ok(())), None) => Err(ConnectionWaitError::SessionClosed),
        };
        if result.is_err() && opened.close_if_original(&self.seat, self.window, cx) {
            let _ = thread.update(cx, |thread, _| thread.forget_background_open(opened.item));
        }
        result
    }
    fn create(
        &self,
        thread: &WeakEntity<AgentThread>,
        cx: &mut AsyncApp,
    ) -> Result<OpenedBackground, ConnectionWaitError> {
        cx.update_window(self.window, |_, window, cx| {
            // Validate immediately before creation in the same application transaction.
            thread
                .update(cx, |thread, cx| {
                    thread.authorize_background_open(
                        &self.seat,
                        self.ticket,
                        self.registration,
                        &self.profile,
                        cx,
                    )
                })
                .map_err(|_| ConnectionWaitError::ChatClosed)??;
            let item = self
                .seat
                .open_background(&self.profile, window, cx)
                .ok_or(ConnectionWaitError::OpenFailed)?;
            if !self.seat.is_open() {
                return Err(ConnectionWaitError::WorkspaceClosed);
            }
            // Capture from placement authority, not the chat's mutable attachment projection.
            let expected = current_session(&self.seat, item, cx);
            Ok(OpenedBackground { item, expected })
        })
        .map_err(|_| ConnectionWaitError::WorkspaceClosed)?
    }
}

/// Missing state represents a malformed provider; it must not orphan its newly owned Item.
struct OpenedBackground {
    item: gpui_kit::EntityId,
    expected: Option<ExpectedSession>,
}
impl OpenedBackground {
    fn close_if_original(&self, seat: &Seat, window: AnyWindowHandle, cx: &mut AsyncApp) -> bool {
        // Membership and epoch are checked in the same workspace transition that closes it.
        cx.update_window(window, |_, window, cx| {
            seat.close_if_original(self.item, self.expected, window, cx)
        })
        .unwrap_or(false)
    }
}
