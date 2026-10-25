//! The application's side of the user plugin host: whether this run loads user plugins at all,
//! where plugin documents live on disk, the host entity and its adapters, the plugin set the
//! windows follow, and the providers of one window.
//!
//! One gate covers every load path. A run holds the user plugins when it was started with
//! `--safe-mode` (always wins) or when `[plugins] user_plugins` is not set; a held run opens no
//! records file, data or package store, creates no runtime or host, restores nothing, and every
//! window attaches nothing. The installed-plugin start and the one-file developer session
//! (`nocterm plugin dev`, which the developer asked for on the command line and which loads
//! nothing that is installed) are the only constructions of the host.
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
use nocterm_plugin_enablement::{PluginEnablement, UserPluginsChoice};
use nocterm_plugin_host::PluginHost;
use nocterm_plugin_model::{PluginCatalog, PluginSet};
use nocterm_plugins_ui::{HeldBy, UserSession};
use nocterm_store::{SettingsHandle, ShutdownCoordinator};
use nocterm_ui::{
    PluginSetHandle, PluginSettingsDirectory, PluginSource, SettingsPageSpec, StaticPluginSource,
};
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

/// How a run starts its user plugins, decided once before anything is opened.
enum Start {
    /// No user plugin loads, and nothing is opened.
    Held(HeldBy),
    /// The installed plugins load from the state directory.
    Installed,
    /// The one file a developer session loads on request; nothing installed loads.
    Dev(PathBuf),
}

/// Safe mode wins over the switch; a `[plugins]` table that does not read leaves the switch off
/// and says why. A developer session is not held by the switch: its one file is loaded on
/// request, after a consent, and nothing installed loads.
fn start_of(mode: UserPluginMode, settings: &SettingsHandle, cx: &App) -> Start {
    match mode {
        UserPluginMode::SafeMode => Start::Held(HeldBy::SafeMode),
        UserPluginMode::Dev(file) => Start::Dev(file),
        UserPluginMode::Installed => {
            match PluginEnablement::user_plugins_in(settings.document(cx)) {
                UserPluginsChoice::Allowed => Start::Installed,
                UserPluginsChoice::Off => Start::Held(HeldBy::NotAllowed),
                UserPluginsChoice::Unreadable { reason } => {
                    Start::Held(HeldBy::Unreadable { reason })
                }
            }
        }
    }
}

/// The user plugins of the run: the host and the plugin set it publishes, or nothing when the
/// run holds them.
#[derive(Clone)]
pub(crate) struct UserShell {
    state: ShellState,
}

#[derive(Clone)]
enum ShellState {
    Loaded(Loaded),
    Held(HeldBy),
}

/// What a run that loads user plugins keeps.
#[derive(Clone)]
struct Loaded {
    host: Entity<PluginHost>,
    source: PluginSetHandle,
    clock: Arc<dyn WallClock>,
    /// The file a developer session loads; `None` for an installation.
    dev: Option<PathBuf>,
}

impl UserShell {
    /// Decides whether the run loads user plugins, then opens them and follows the host: the
    /// windows read the set it publishes. An installation opens the stored plugins; a
    /// developer session starts empty over memory-only ports. A held run opens nothing: no
    /// file, no host, no runtime.
    pub(crate) fn install(
        catalog: &Arc<PluginCatalog>,
        paths: &Paths,
        shutdown: &ShutdownCoordinator,
        clock: Arc<dyn WallClock>,
        mode: UserPluginMode,
        settings: &SettingsHandle,
        cx: &mut App,
    ) -> Self {
        settings.register::<PluginEnablement>(cx);
        let (ports, dev) = match start_of(mode, settings, cx) {
            Start::Held(by) => {
                return Self {
                    state: ShellState::Held(by),
                };
            }
            Start::Dev(file) => (ports::session(catalog), Some(file)),
            Start::Installed => {
                let root = paths.state_dir().join("plugins");
                (ports::installed(&root, catalog, shutdown, cx), None)
            }
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
            state: ShellState::Loaded(Loaded {
                host,
                source,
                clock,
                dev,
            }),
        }
    }

    /// The host, for tests that install plugins into it. A held run has none.
    #[cfg(test)]
    pub(crate) fn host(&self) -> Option<&Entity<PluginHost>> {
        match &self.state {
            ShellState::Loaded(loaded) => Some(&loaded.host),
            ShellState::Held(_) => None,
        }
    }

    /// Why this run loads no user plugins, if it does not.
    #[cfg(test)]
    pub(crate) fn held_by(&self) -> Option<HeldBy> {
        match &self.state {
            ShellState::Loaded(_) => None,
            ShellState::Held(by) => Some(by.clone()),
        }
    }

    /// What the windows read the plugin set from: the host's, or the system plugins alone when
    /// the run holds the user plugins.
    pub(crate) fn source(&self, catalog: &Arc<PluginCatalog>) -> Rc<dyn PluginSource> {
        match &self.state {
            ShellState::Loaded(loaded) => Rc::new(loaded.source.clone()),
            ShellState::Held(_) => {
                Rc::new(StaticPluginSource::new(PluginSet::system(catalog.clone())))
            }
        }
    }

    /// `system` and the settings of the installed user plugins.
    pub(crate) fn directory(&self, system: PluginSettingsDirectory) -> PluginSettingsDirectory {
        match &self.state {
            ShellState::Loaded(loaded) => {
                system.with_user(Rc::new(settings::HostSettings::new(loaded.host.clone())))
            }
            ShellState::Held(_) => system,
        }
    }

    /// What the editor of the bar shows of a selected user plugin.
    pub(crate) fn facts(&self) -> Option<Rc<dyn nocterm_layout_ui::UserPluginFacts>> {
        match &self.state {
            ShellState::Loaded(loaded) => Some(Rc::new(facts::HostFacts::new(loaded.host.clone()))),
            ShellState::Held(_) => None,
        }
    }

    /// The Plugins page with its User section. `toggles` is also the switch for the next start.
    pub(crate) fn plugins_page(
        &self,
        catalog: &Arc<PluginCatalog>,
        toggles: Rc<AppPluginToggles>,
        directory: PluginSettingsDirectory,
    ) -> SettingsPageSpec {
        let session = match &self.state {
            ShellState::Loaded(loaded) => UserSession::Loaded(Rc::new(manager::HostPlugins::new(
                loaded.host.clone(),
                loaded.dev.clone(),
            ))),
            ShellState::Held(by) => UserSession::Held(by.clone()),
        };
        nocterm_plugins_ui::settings_page_with_user_plugins(
            catalog.clone(),
            toggles.clone(),
            directory,
            session,
            toggles,
        )
    }

    pub(crate) fn window(&self, files: &nocterm_files::Installed) -> WindowUser {
        WindowUser {
            loaded: match &self.state {
                ShellState::Loaded(shell) => Some(LoadedWindow {
                    shell: shell.clone(),
                    usage: MonitorUsage::new(),
                    counts: TransferCounts::new(files),
                }),
                ShellState::Held(_) => None,
            },
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

/// What one window keeps of the user plugins: nothing in a held run.
pub(crate) struct WindowUser {
    loaded: Option<LoadedWindow>,
}

/// The monitor reading its plugins may observe, and the transfer counts.
struct LoadedWindow {
    shell: Loaded,
    usage: MonitorUsage,
    counts: TransferCounts,
}

impl WindowUser {
    /// Registers the monitor; when the run loads user plugins, plugins granted `monitor.usage`
    /// can read what it shows.
    pub(crate) fn register_monitor(
        &self,
        installed: &nocterm_monitor_ui::Installed,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), RegistrationError> {
        match &self.loaded {
            Some(loaded) => nocterm_monitor_ui::register_observed(
                installed,
                &loaded.usage,
                workspace,
                window,
                cx,
            ),
            None => nocterm_monitor_ui::register(installed, workspace, window, cx),
        }
    }

    /// Registers the user contributions of the plugin set and follows it for the life of the
    /// window. A held run attaches nothing.
    pub(crate) fn attach(
        self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let Some(LoadedWindow {
            shell,
            usage,
            counts,
        }) = self.loaded
        else {
            return;
        };
        if let Some(file) = &shell.dev {
            announce_dev(file, window, cx);
        }
        let this = cx.entity();
        let badges = Badges::default();
        let providers = window_providers(
            &this,
            window.window_handle(),
            &usage,
            counts,
            shell.clock.clone(),
            &badges,
            cx,
        );
        let sync = WindowSync::new(
            shell.host,
            shell.source,
            providers,
            badges,
            shell.dev.is_some(),
            window.window_handle().window_id(),
        );
        super::user_plugin_panel::attach(sync, workspace, window, cx);
    }
}

#[cfg(test)]
mod dev_session;
#[cfg(test)]
mod gate;
#[cfg(test)]
mod rust_guest;
#[cfg(test)]
mod tests;
