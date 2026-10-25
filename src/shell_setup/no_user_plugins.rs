//! The default build has no user plugins: the plugin set never changes and no window attaches
//! anything.
use std::{rc::Rc, sync::Arc};

use gpui_kit::{App, Context, Window};
use nocterm_core::Paths;
use nocterm_plugin_model::{PluginCatalog, PluginSet};
use nocterm_store::ShutdownCoordinator;
use nocterm_ui::{PluginSettingsDirectory, PluginSource, SettingsPageSpec, StaticPluginSource};
use nocterm_workspace::{RegistrationError, Workspace};

use super::AppPluginToggles;

/// Nothing to compose without user plugins.
#[derive(Clone)]
pub(crate) struct UserShell;

impl UserShell {
    pub(crate) fn install(
        _catalog: &Arc<PluginCatalog>,
        _paths: &Paths,
        _shutdown: &ShutdownCoordinator,
        _cx: &mut App,
    ) -> Self {
        Self
    }

    /// The system plugins only, so the set never changes.
    pub(crate) fn source(&self, catalog: &Arc<PluginCatalog>) -> Rc<dyn PluginSource> {
        Rc::new(StaticPluginSource::new(PluginSet::system(catalog.clone())))
    }

    pub(crate) fn directory(&self, system: PluginSettingsDirectory) -> PluginSettingsDirectory {
        system
    }

    pub(crate) fn plugins_page(
        &self,
        catalog: &Arc<PluginCatalog>,
        toggles: Rc<AppPluginToggles>,
        directory: PluginSettingsDirectory,
    ) -> SettingsPageSpec {
        nocterm_plugins_ui::settings_page(catalog.clone(), toggles, directory)
    }

    pub(crate) fn window(&self, _files: &nocterm_files::Installed) -> WindowUser {
        WindowUser
    }
}

/// What one window keeps of the user plugins: nothing.
pub(crate) struct WindowUser;

impl WindowUser {
    pub(crate) fn register_monitor(
        &self,
        installed: &nocterm_monitor_ui::Installed,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), RegistrationError> {
        nocterm_monitor_ui::register(installed, workspace, window, cx)
    }

    pub(crate) fn attach(
        self,
        _workspace: &mut Workspace,
        _window: &mut Window,
        _cx: &mut Context<Workspace>,
    ) {
    }
}
