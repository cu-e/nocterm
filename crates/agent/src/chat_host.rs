//! Explicit immutable host collaborators retained by each panel and its chats.
use gpui_kit::{App, WeakEntity};
use nocterm_workspace::Workspace;
#[derive(Clone)]
pub(crate) struct ChatHost {
    workspace: WeakEntity<Workspace>,
    installed: crate::Installed,
}
impl ChatHost {
    pub(crate) fn new(workspace: WeakEntity<Workspace>, installed: crate::Installed) -> Self {
        Self {
            workspace,
            installed,
        }
    }
    pub(crate) fn workspace(&self) -> &WeakEntity<Workspace> {
        &self.workspace
    }
    pub(crate) fn runtime(&self) -> &crate::runtime::RuntimeHandle {
        self.installed.runtime()
    }
    pub(crate) fn retention<'a>(
        &'a self,
        cx: &App,
    ) -> &'a nocterm_ai::model::retention::RetentionAuthority {
        self.runtime().retention(cx)
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
    pub(crate) fn new_identity(&self) -> nocterm_ai::metadata::ChatIdentity {
        self.installed.new_identity()
    }
}
