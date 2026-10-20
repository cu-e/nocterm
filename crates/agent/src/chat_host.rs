//! Explicit immutable host collaborators retained by each panel.
use crate::chat_deps::ChatDeps;
use gpui_kit::WeakEntity;
use nocterm_workspace::Workspace;
use std::rc::Rc;
#[derive(Clone)]
pub(crate) struct ChatHost {
    workspace: WeakEntity<Workspace>,
    deps: ChatDeps,
    installed: crate::Installed,
}
impl ChatHost {
    pub(crate) fn new(workspace: WeakEntity<Workspace>, installed: crate::Installed) -> Self {
        let seat = Rc::new(crate::workspace_seat::WorkspaceSeat::new(workspace.clone()));
        let sign_in = installed.auth().map(|opener| {
            Rc::new(crate::workspace_sign_in::WorkspaceSignIn::new(
                opener,
                workspace.clone(),
            )) as Rc<dyn crate::sign_in::SignInTerminal>
        });
        Self {
            deps: ChatDeps::new(
                installed.runtime().clone(),
                installed.agents().clone(),
                installed.settings().clone(),
                installed.metadata(),
                seat,
                sign_in,
            ),
            workspace,
            installed,
        }
    }
    /// The panel's workspace; chats use the seat in [`Self::deps`].
    pub(crate) fn workspace(&self) -> &WeakEntity<Workspace> {
        &self.workspace
    }
    /// What the panel's chats receive.
    pub(crate) fn deps(&self) -> &ChatDeps {
        &self.deps
    }
    pub(crate) fn sign_in(&self) -> Option<&Rc<dyn crate::sign_in::SignInTerminal>> {
        self.deps.sign_in()
    }
    pub(crate) fn runtime(&self) -> &crate::runtime::RuntimeHandle {
        self.deps.runtime()
    }
    pub(crate) fn agents(&self) -> &nocterm_agent_directory::AgentsHandle {
        self.deps.agents()
    }
    pub(crate) fn settings(&self) -> &nocterm_store::SettingsHandle {
        self.deps.settings()
    }
    pub(crate) fn images(&self) -> &crate::ImageDeps {
        self.installed.images()
    }
    pub(crate) fn layout(&self) -> &nocterm_ui::LayoutHandle {
        self.installed.layout()
    }
    pub(crate) fn timestamp(&self) -> u64 {
        self.deps.timestamp()
    }
}
