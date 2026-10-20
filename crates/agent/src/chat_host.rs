//! Explicit immutable host collaborators retained by each panel and its chats.
use gpui_kit::{App, WeakEntity};
use nocterm_ai::history::SavedChat;
use nocterm_workspace::Workspace;
#[derive(Clone)]
pub(crate) struct ChatHost {
    workspace: WeakEntity<Workspace>,
    seat: crate::seat::Seat,
    installed: crate::Installed,
}
impl ChatHost {
    pub(crate) fn new(workspace: WeakEntity<Workspace>, installed: crate::Installed) -> Self {
        Self {
            seat: std::rc::Rc::new(crate::seat::WorkspaceSeat::new(workspace.clone())),
            workspace,
            installed,
        }
    }
    /// The panel's workspace; chats use [`Self::seat`].
    pub(crate) fn workspace(&self) -> &WeakEntity<Workspace> {
        &self.workspace
    }
    /// What chats may do with the window.
    pub(crate) fn seat(&self) -> &crate::seat::Seat {
        &self.seat
    }
    /// Opens `request`'s sign-in terminal in this window, when the host can.
    pub(crate) fn sign_in(
        &self,
        request: crate::TerminalAuthRequest,
        window: &mut gpui_kit::Window,
        cx: &mut App,
    ) -> Option<
        futures::channel::oneshot::Receiver<Result<(), crate::TerminalAuthError>>,
    > {
        let opener = self.installed.auth()?;
        Some(opener(self.workspace.clone(), request, window, cx))
    }
    pub(crate) fn runtime(&self) -> &crate::runtime::RuntimeHandle {
        self.installed.runtime()
    }
    pub(crate) fn agents(&self) -> &nocterm_agent_directory::AgentsHandle {
        self.installed.agents()
    }
    pub(crate) fn settings(&self) -> &nocterm_store::SettingsHandle {
        self.installed.settings()
    }
    pub(crate) fn ai<'a>(&self, cx: &'a App) -> &'a nocterm_ai::AiSettings {
        self.settings().setting::<nocterm_ai::AiSettings>(cx)
    }
    pub(crate) fn enabled(&self, cx: &App) -> bool {
        self.ai(cx).enabled
    }
    pub(crate) fn images(&self) -> &crate::ImageDeps {
        self.installed.images()
    }
    pub(crate) fn layout(&self) -> &nocterm_ui::LayoutHandle {
        self.installed.layout()
    }
    pub(crate) fn auth(&self) -> Option<crate::TerminalAuthOpener> {
        self.installed.auth()
    }
    pub(crate) fn timestamp(&self) -> u64 {
        self.installed.timestamp()
    }
    pub(crate) fn new_chat(&self, agent: String) -> SavedChat {
        self.installed.new_chat(agent)
    }
}
