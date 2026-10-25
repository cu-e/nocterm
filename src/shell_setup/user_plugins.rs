//! The application's side of the user plugin host: where plugin documents live on disk, the
//! host entity and its adapters, the plugin set the windows follow, and the providers of one
//! window.
//!
//! Plugin documents live under `<state dir>/plugins`: the records file, one data file per
//! plugin and one package file per plugin.
use std::{rc::Rc, sync::Arc};

use gpui_kit::{App, AppContext as _, Context, Entity, Window};
use nocterm_core::{Paths, fs::BoundedReadError};
use nocterm_files::TransferCounts;
use nocterm_monitor_ui::MonitorUsage;
use nocterm_plugin_api::DeclarativeRuntime;
use nocterm_plugin_host::{HostPorts, PluginHost};
use nocterm_plugin_model::{PluginCatalog, PluginSet};
use nocterm_plugin_packages::PackageStore;
use nocterm_store::ShutdownCoordinator;
use nocterm_ui::{PluginSetHandle, PluginSettingsDirectory, PluginSource, SettingsPageSpec};
use nocterm_workspace::{RegistrationError, Workspace};

use super::AppPluginToggles;

mod check;
mod data;
mod manager;
mod providers;
mod records;
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
}

impl UserShell {
    /// Opens the stored plugins and follows the host: the windows read the set it publishes.
    pub(crate) fn install(
        catalog: &Arc<PluginCatalog>,
        paths: &Paths,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Self {
        let root = paths.state_dir().join("plugins");
        let records = RecordsFile::open(root.join("records.json"), cx);
        let data = PluginDataFiles::new(root.join("data"), cx);
        for (owner, registered) in [
            ("plugin records", records.register_shutdown(shutdown)),
            ("plugin data", data.register_shutdown(shutdown)),
        ] {
            if let Err(error) = registered {
                tracing::error!(owner, %error, "shutdown registration failed");
            }
        }
        let ports = HostPorts {
            records: Rc::new(records),
            data: Rc::new(data),
            packages: Arc::new(PackageStore::new(
                root.join("packages"),
                reserved_names(catalog),
            )),
            runtime: Arc::new(DeclarativeRuntime),
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
        Self { host, source }
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
            Rc::new(manager::HostPlugins::new(self.host.clone())),
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
        let this = cx.entity();
        let providers =
            window_providers(&this, window.window_handle(), &self.usage, self.counts, cx);
        super::user_plugin_panel::attach(
            self.shell.host,
            self.shell.source,
            providers,
            workspace,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests;
