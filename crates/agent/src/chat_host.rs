//! Explicit immutable host collaborators retained by each panel and its chats.
use gpui_kit::{App, WeakEntity};
use nocterm_ai::history::SavedChat;
use nocterm_workspace::Workspace;
#[derive(Clone)]
pub(crate) struct ChatHost {
    workspace: WeakEntity<Workspace>,
    seat: crate::seat::Seat,
    sign_in: Option<std::rc::Rc<dyn crate::sign_in::SignInTerminal>>,
    installed: crate::Installed,
}
impl ChatHost {
    pub(crate) fn new(workspace: WeakEntity<Workspace>, installed: crate::Installed) -> Self {
        Self {
            seat: std::rc::Rc::new(crate::seat::WorkspaceSeat::new(workspace.clone())),
            sign_in: installed.auth().map(|opener| {
                std::rc::Rc::new(crate::sign_in::WorkspaceSignIn::new(opener, workspace.clone()))
                    as std::rc::Rc<dyn crate::sign_in::SignInTerminal>
            }),
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
    /// Runs sign-in terminals in this window, when the host can.
    pub(crate) fn sign_in(&self) -> Option<&std::rc::Rc<dyn crate::sign_in::SignInTerminal>> {
        self.sign_in.as_ref()
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
    pub(crate) fn timestamp(&self) -> u64 {
        self.installed.timestamp()
    }
    pub(crate) fn new_chat(&self, agent: String) -> SavedChat {
        self.installed.new_chat(agent)
    }
}
