//! Explicit immutable host collaborators retained by each panel.
use crate::chat_deps::ChatDeps;
use gpui_kit::WeakEntity;
use nocterm_workspace::{ProjectSurfaceReceipt, Workspace};
use std::rc::Rc;
#[derive(Clone)]
pub(crate) struct ChatHost {
    workspace: WeakEntity<Workspace>,
    deps: ChatDeps,
    installed: crate::Installed,
    source: ProjectSurfaceReceipt,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum ChatHostError {
    #[error("The originating root window no longer matches this chat host.")]
    WindowMismatch,
}
impl ChatHost {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        installed: crate::Installed,
        source: ProjectSurfaceReceipt,
    ) -> Result<Self, ChatHostError> {
        if source.window().workspace_id() != workspace.entity_id() {
            return Err(ChatHostError::WindowMismatch);
        }
        let seat = Rc::new(crate::workspace_seat::WorkspaceSeat::new(
            workspace.clone(),
            source,
        ));
        let sign_in = installed.auth().map(|opener| {
            Rc::new(crate::workspace_sign_in::WorkspaceSignIn::new(
                opener,
                workspace.clone(),
            )) as Rc<dyn crate::sign_in::SignInTerminal>
        });
        Ok(Self {
            deps: ChatDeps::new(
                installed.runtime().for_scope(source.context()),
                installed.agents().clone(),
                installed.settings().clone(),
                installed.metadata(),
                seat,
                sign_in,
            ),
            workspace,
            installed,
            source,
        })
    }
    pub(crate) fn source(&self) -> ProjectSurfaceReceipt {
        self.source
    }
    /// Capture a committed UI origin synchronously. This supplies descriptive
    /// membership; terminals remain attached to that original ordinary surface.
    pub(crate) fn deps_for_source(
        &self,
        source: ProjectSurfaceReceipt,
    ) -> Result<ChatDeps, ChatHostError> {
        if source.window().workspace_id() != self.workspace.entity_id() {
            return Err(ChatHostError::WindowMismatch);
        }
        Ok(ChatDeps::new(
            self.installed.runtime().for_scope(source.context()),
            self.installed.agents().clone(),
            self.installed.settings().clone(),
            self.installed.metadata(),
            Rc::new(crate::workspace_seat::WorkspaceSeat::new(
                self.workspace.clone(),
                source,
            )),
            self.deps.sign_in().cloned(),
        ))
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
