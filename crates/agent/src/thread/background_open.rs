//! A background opening owns its original workspace receipt independently of chat lifetime.
use super::{
    AgentThread,
    connection_wait::ConnectionWaitError,
    tools::{WaitTarget, wait_until_connected},
};
use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, WeakEntity};
use nocterm_ui::ActiveAi as _;
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
    fn authorize(&self, thread: &AgentThread, cx: &App) -> Result<(), ConnectionWaitError> {
        if thread.workspace.entity_id() != self.workspace.entity_id()
            || thread.registration().as_ref().map(|value| value.id) != self.registration
            || !thread.lifecycle.session_current(self.ticket)
            || !thread.lifecycle.accepts_updates()
            || !cx.ai_enabled()
        {
            return Err(ConnectionWaitError::Stopped);
        }
        if !thread.server_is_attached(&self.profile, cx) {
            return Err(ConnectionWaitError::Detached);
        }
        Ok(())
    }
    pub(super) async fn open(
        &self,
        thread: &WeakEntity<AgentThread>,
        cx: &mut AsyncApp,
    ) -> Result<serde_json::Value, ConnectionWaitError> {
        let opened = self.create(thread, cx)?;
        let registered = thread
            .update(cx, |thread, cx| {
                self.authorize(thread, cx)?;
                thread.background.push(opened.item);
                Ok(())
            })
            .map_err(|_| ConnectionWaitError::ChatClosed);
        let result = match (registered, opened.expected) {
            (Ok(Ok(())), Some(expected)) => {
                wait_until_connected(
                    thread,
                    &WaitTarget::new(
                        self.workspace.clone(),
                        self.window,
                        expected,
                        self.ticket,
                        self.registration,
                    ),
                    cx,
                )
                .await
            }
            (Ok(Err(error)) | Err(error), _) => Err(error),
            (Ok(Ok(())), None) => Err(ConnectionWaitError::SessionClosed),
        };
        if result.is_err() && opened.close_if_original(&self.workspace, self.window, cx) {
            let _ = thread.update(cx, |thread, _| {
                thread.background.retain(|item| *item != opened.item)
            });
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
                .update(cx, |thread, cx| self.authorize(thread, cx))
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
