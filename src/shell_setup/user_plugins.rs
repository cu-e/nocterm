//! The application's side of the user plugin host: where plugin documents live on disk, the
//! host entity and its adapters, the plugin set the windows follow, and the providers of one
//! window.
//!
//! Plugin documents live under `<state dir>/plugins`: the records file, one data file per
//! plugin and one package file per plugin. A developer session (`nocterm plugin dev <file>`)
//! composes memory-only ports instead and touches none of it; `startup` also keeps the session's
//! settings in memory (`UserPluginMode::saves_settings`).
use std::{path::PathBuf, rc::Rc, sync::Arc};

use gpui_kit::{App, AppContext as _, Context, Entity, Window};
use nocterm_core::{Paths, clock::WallClock, fs::BoundedReadError};
use nocterm_files::TransferCounts;
use nocterm_monitor_ui::MonitorUsage;
use nocterm_plugin_host::PluginHost;
use nocterm_plugin_model::{PluginCatalog, PluginSet};
use nocterm_store::ShutdownCoordinator;
use nocterm_ui::{PluginSetHandle, PluginSettingsDirectory, PluginSource, SettingsPageSpec};
use nocterm_workspace::{RegistrationError, Workspace};

use super::{
    AppPluginToggles, UserPluginMode,
    user_plugin_panel::{Badges, WindowSync},
};

mod check;
mod data;
mod facts;
mod manager;
mod ports;
mod providers;
mod records;
mod runtime;
mod settings;

pub(crate) use check::check_directory;
pub(crate) use data::PluginDataFiles;
pub(crate) use providers::window_providers;
pub(crate) use records::RecordsFile;

/// Whether a failed bounded read means the file is not there at all.
fn is_missing(error: &BoundedReadError) -> bool {
    error.io_kind() == Some(std::io::ErrorKind::NotFound)
}

/// The names a package may not take: the system plugins, the built-in contributions and the
/// panel groups.
fn reserved_names(catalog: &PluginCatalog) -> Vec<String> {
    let plugins = catalog.plugins().map(|plugin| plugin.name.to_owned());
    let titles = catalog.entries().map(|entry| entry.title.to_owned());
    let groups = catalog.groups().iter().map(|group| group.title.to_owned());
    plugins.chain(titles).chain(groups).collect()
}

/// The user plugins of the installation: the host entity and the plugin set it publishes.
#[derive(Clone)]
pub(crate) struct UserShell {
    host: Entity<PluginHost>,
    source: PluginSetHandle,
    clock: Arc<dyn WallClock>,
    /// The file a developer session loads; `None` for an installation.
    dev: Option<PathBuf>,
}

impl UserShell {
    /// Opens the plugins of the mode and follows the host: the windows read the set it
    /// publishes. An installation opens the stored plugins; a developer session starts empty
    /// over memory-only ports.
    pub(crate) fn install(
        catalog: &Arc<PluginCatalog>,
        paths: &Paths,
        shutdown: &ShutdownCoordinator,
        clock: Arc<dyn WallClock>,
        mode: UserPluginMode,
        cx: &mut App,
    ) -> Self {
        let (ports, dev) = match mode {
            UserPluginMode::Installed => {
                let root = paths.state_dir().join("plugins");
                (ports::installed(&root, catalog, shutdown, cx), None)
            }
            UserPluginMode::Dev(file) => (ports::session(catalog), Some(file)),
        };
        let host = cx.new(|_| PluginHost::new(ports));
        host.update(cx, |host, cx| host.restore(cx));
        let base = PluginSet::system(catalog.clone());
        let source = PluginSetHandle::new(published(&base, &host, cx), cx);
        let follower = source.clone();
        cx.observe(&host, move |host, cx| {
            let next = published(&base, &host, cx);
            if follower.current(cx).users() != next.users() {
                follower.replace(next, cx);
            }
        })
        .detach();
        Self {
            host,
            source,
            clock,
            dev,
        }
    }

    /// The host, for tests that install plugins into it.
    #[cfg(test)]
    pub(crate) fn host(&self) -> &Entity<PluginHost> {
        &self.host
    }

    pub(crate) fn source(&self, _catalog: &Arc<PluginCatalog>) -> Rc<dyn PluginSource> {
        Rc::new(self.source.clone())
    }

    /// `system` and the settings of the installed user plugins.
    pub(crate) fn directory(&self, system: PluginSettingsDirectory) -> PluginSettingsDirectory {
        system.with_user(Rc::new(settings::HostSettings::new(self.host.clone())))
    }

    /// What the editor of the bar shows of a selected user plugin.
    pub(crate) fn facts(&self) -> Option<Rc<dyn nocterm_layout_ui::UserPluginFacts>> {
        Some(Rc::new(facts::HostFacts::new(self.host.clone())))
    }

    /// The Plugins page with its User section.
    pub(crate) fn plugins_page(
        &self,
        catalog: &Arc<PluginCatalog>,
        toggles: Rc<AppPluginToggles>,
        directory: PluginSettingsDirectory,
    ) -> SettingsPageSpec {
        nocterm_plugins_ui::settings_page_with_user_plugins(
            catalog.clone(),
            toggles,
            directory,
            Rc::new(manager::HostPlugins::new(
                self.host.clone(),
                self.dev.clone(),
            )),
        )
    }

    pub(crate) fn window(&self, files: &nocterm_files::Installed) -> WindowUser {
        WindowUser {
            shell: self.clone(),
            usage: MonitorUsage::new(),
            counts: TransferCounts::new(files),
        }
    }
}

/// The set the host publishes now. A set the shell refuses leaves only the system plugins.
fn published(base: &PluginSet, host: &Entity<PluginHost>, cx: &App) -> PluginSet {
    let host = host.read(cx);
    base.with_users(host.entries(), host.revision())
        .unwrap_or_else(|error| {
            tracing::error!(%error, "the installed user plugins do not form a valid set");
            base.clone()
        })
}

/// Tells the user, in every window, that this is a developer session and where to load the
/// plugin under test.
fn announce_dev(file: &std::path::Path, window: &mut Window, cx: &mut App) {
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    nocterm_ui::notice::warning_action(
        window,
        cx,
        "plugin-dev",
        "Developer session",
        format!("Settings and the plugin are not saved. Load {name} from Settings, Plugins."),
        "Open settings",
        |window, cx| window.dispatch_action(Box::new(nocterm_workspace::OpenSettings), cx),
    );
}

/// What one window keeps of the user plugins: the monitor reading its plugins may observe, and
/// the transfer counts.
pub(crate) struct WindowUser {
    shell: UserShell,
    usage: MonitorUsage,
    counts: TransferCounts,
}

impl WindowUser {
    /// Registers the monitor so that plugins granted `monitor.usage` can read what it shows.
    pub(crate) fn register_monitor(
        &self,
        installed: &nocterm_monitor_ui::Installed,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), RegistrationError> {
        nocterm_monitor_ui::register_observed(installed, &self.usage, workspace, window, cx)
    }

    /// Registers the user contributions of the plugin set and follows it for the life of the
    /// window.
    pub(crate) fn attach(
        self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if let Some(file) = &self.shell.dev {
            announce_dev(file, window, cx);
        }
        let this = cx.entity();
        let badges = Badges::default();
        let providers = window_providers(
            &this,
            window.window_handle(),
            &self.usage,
            self.counts,
            self.shell.clock.clone(),
            &badges,
            cx,
        );
        let sync = WindowSync::new(
            self.shell.host,
            self.shell.source,
            providers,
            badges,
            self.shell.dev.is_some(),
            window.window_handle().window_id(),
        );
        super::user_plugin_panel::attach(sync, workspace, window, cx);
    }
}

#[cfg(test)]
mod dev_session;
#[cfg(test)]
mod rust_guest;
#[cfg(test)]
mod tests;
