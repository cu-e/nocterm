//! A background opening owns its original workspace receipt independently of chat lifetime.
use super::{
    AgentThread,
    connection_wait::ConnectionWaitError,
    connection_wait::{ConnectionWaitOperation, WaitTarget},
};
use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, WeakEntity};
use nocterm_workspace::{ConnectionDirectory, ExpectedSession, Workspace};
use std::rc::Rc;

pub(super) struct BackgroundOpen {
    workspace: WeakEntity<Workspace>,
    window: AnyWindowHandle,
    directory: Rc<dyn ConnectionDirectory>,
    profile: String,
    ticket: nocterm_ai::session::Ticket,
    registration: Option<u64>,
}
impl BackgroundOpen {
    pub(super) fn workspace(&self) -> &WeakEntity<Workspace> {
        &self.workspace
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
        workspace: WeakEntity<Workspace>,
        window: AnyWindowHandle,
        directory: Rc<dyn ConnectionDirectory>,
        profile: String,
        ticket: nocterm_ai::session::Ticket,
        registration: Option<u64>,
    ) -> Self {
        Self {
            workspace,
            window,
            directory,
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
                    self.workspace.clone(),
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
        if result.is_err() && opened.close_if_original(&self.workspace, self.window, cx) {
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
                        &self.workspace,
                        self.ticket,
                        self.registration,
                        &self.profile,
                        cx,
                    )
                })
                .map_err(|_| ConnectionWaitError::ChatClosed)??;
            let item = self
                .directory
                .open_background(&self.profile, &self.workspace, window, cx)
                .ok_or(ConnectionWaitError::OpenFailed)?;
            let workspace = self
                .workspace
                .upgrade()
                .ok_or(ConnectionWaitError::WorkspaceClosed)?;
            // Capture from placement authority, not the chat's mutable attachment projection.
            let expected = OpenedBackground::current(workspace.read(cx), item, cx);
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
    fn current(
        workspace: &Workspace,
        item: gpui_kit::EntityId,
        cx: &App,
    ) -> Option<ExpectedSession> {
        workspace
            .terminals(cx)
            .into_iter()
            .find(|entry| entry.item == item)
            .and_then(|entry| entry.capabilities.state().info(cx))
            .map(|info| info.expected)
    }
    fn close_if_original(
        &self,
        workspace: &WeakEntity<Workspace>,
        window: AnyWindowHandle,
        cx: &mut AsyncApp,
    ) -> bool {
        // Membership and epoch are checked in the same workspace transition that closes it.
        cx.update_window(window, |_, window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    if !workspace.is_background(self.item)
                        || Self::current(workspace, self.item, cx) != self.expected
                    {
                        return false;
                    }
                    workspace.close_background_session(self.item, window, cx);
                    true
                })
                .unwrap_or(false)
        })
        .unwrap_or(false)
    }
}
