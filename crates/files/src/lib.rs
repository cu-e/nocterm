//! Explorer navigation, file dialogs and transfer presentation.
//! Browser generations, scan results and completion cursors remain private transition owners.
//!
//! Owns: independent pane views, path editors, selection and pinned operation dialogs.
//! Does not own: native filesystem traversal/mutations, transfer execution or process spawning.
//! Ports used: `LocalFs`, `LocalMutations`, `RemoteFs`, `TransferService`, `ProgramOpener`,
//! workspace `LocalTerminal` and the retained settings service handle.
//! Ports provided: `Panel`, `SettingsPageSpec`. Settings section: explorer. Globals: none.
//! Invariants: native paths keep their original names; each pane rejects stale navigation;
//! accepted operations retain their original capability while later navigation proceeds.
mod activity;
mod dialogs;
mod local;
mod local_pane;
mod open;
mod operations;
mod path_input;
mod registration;
mod remote;
mod remote_pane;
mod row;
mod settings;
mod settings_page;
mod statistics;
#[cfg(test)]
mod tests;
mod transfers;

use gpui_kit::{
    AnyElement, AnyWindowHandle, App, Context, Entity, ExternalPaths, FocusHandle, Focusable,
    MouseButton, SharedString, Subscription, Task, WeakEntity, Window,
    component::{
        ActiveTheme as _, Disableable as _, ResizableState, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        menu::ContextMenuExt as _,
        resizable_panel, v_flex, v_resizable,
    },
    div,
    prelude::*,
    px, uniform_list,
};
#[cfg(test)]
use nocterm_session::{DirEntry, FsError};
use nocterm_session::{EntryKind, RemoteFs, fs::path};
use nocterm_transfers::{CollisionPolicy, DownloadRequest, UploadRequest};
use nocterm_ui::{ActiveDesign as _, IconName};
use nocterm_workspace::{
    FileDrag, Panel, RemoteFileDrag as RemotePaths, SessionContext, Workspace, WorkspaceEvent,
};
use operations::FileTarget;
use remote::{Browser, listing};
use std::{collections::BTreeSet, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

pub use registration::{
    Installed, OpenExplorerSettings, ShowTransfers, ToggleExplorer, install, register,
};
pub use settings::{
    ExplorerSettings, FILE_PLACEHOLDER, INDEXING_ENTRIES_RANGE, IndexingSettings, OpenRule,
    OpenSettings, Opener,
};
pub use settings_page::ExplorerPage;

/// Collaborators supplied by application composition.
#[derive(Clone)]
pub struct Deps {
    local: Arc<dyn nocterm_session::local_fs::LocalFs>,
    mutations: Arc<dyn nocterm_session::local_fs::LocalMutations>,
    transfers: Arc<dyn nocterm_transfers::TransferService>,
    opener: Arc<dyn nocterm_session::ProgramOpener>,
}
impl Deps {
    pub fn new(
        local: Arc<dyn nocterm_session::local_fs::LocalFs>,
        mutations: Arc<dyn nocterm_session::local_fs::LocalMutations>,
        transfers: Arc<dyn nocterm_transfers::TransferService>,
        opener: Arc<dyn nocterm_session::ProgramOpener>,
    ) -> Self {
        Self {
            local,
            mutations,
            transfers,
            opener,
        }
    }
}
/// Supplies the lazy Explorer page to the application Settings host.
pub fn settings_page(installed: &Installed) -> nocterm_ui::SettingsPageSpec {
    let settings = installed.settings().clone();
    nocterm_ui::SettingsPageSpec::new("explorer", "Explorer", move |window, cx| {
        cx.new(|cx| ExplorerPage::new(settings.clone(), window, cx))
    })
    .with_icon(IconName::FolderTree)
}

use gpui_kit::EventEmitter;
use local_pane::LocalPane;
use remote_pane::RemotePane;
#[derive(Clone)]
struct PaneHost {
    installed: Installed,
    workspace: WeakEntity<Workspace>,
    window: AnyWindowHandle,
}
enum PaneIntent {
    Upload {
        sources: Vec<PathBuf>,
        destination: Option<String>,
    },
    Download {
        files: RemotePaths,
        destination: Option<PathBuf>,
    },
    Collision(CollisionPolicy),
    Changed,
}
struct PanelObservers {
    _subscriptions: Vec<Subscription>,
    _tick: Task<()>,
}
pub struct FilesPanel {
    focus: FocusHandle,
    local: Entity<LocalPane>,
    remote: Entity<RemotePane>,
    activity: activity::Activity,
    divider: Entity<ResizableState>,
    collisions: CollisionPolicy,
    window: AnyWindowHandle,
    _observers: PanelObservers,
    installed: Installed,
}
/// A directory listing as presented by Explorer; paths retain native identity.
#[derive(Clone, Debug)]
pub struct ListingState<P> {
    path: Option<P>,
    requested: Option<P>,
    names: Vec<String>,
    selected: Vec<usize>,
    loading: bool,
    error: Option<String>,
    statistics: String,
}
impl<P: Clone> ListingState<P> {
    pub fn path(&self) -> Option<P> {
        self.path.clone()
    }
    pub fn requested(&self) -> Option<P> {
        self.requested.clone()
    }
}
impl<P> ListingState<P> {
    pub fn names(&self) -> &[String] {
        &self.names
    }
    pub fn selected(&self) -> &[usize] {
        &self.selected
    }
    pub fn loading(&self) -> bool {
        self.loading
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn statistics(&self) -> &str {
        &self.statistics
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteAccess {
    NoSession,
    Disconnected,
    Unsupported,
    Available,
}
impl FilesPanel {
    /// Starts local navigation without changing the remote session.
    pub fn navigate_local(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.local
            .update(cx, |local, cx| local.load_local(directory, cx));
    }
    /// Starts remote navigation; `None` resolves the connected host's home.
    pub fn navigate_remote(&mut self, directory: Option<String>, cx: &mut Context<Self>) {
        self.remote
            .update(cx, |remote, cx| remote.load(directory, cx));
    }
    pub fn local_listing(&self, cx: &App) -> ListingState<PathBuf> {
        self.local.read(cx).listing()
    }
    pub fn remote_listing(&self, cx: &App) -> ListingState<String> {
        self.remote.read(cx).listing()
    }
    pub fn remote_access(&self, cx: &App) -> RemoteAccess {
        self.remote.read(cx).access()
    }
    fn new(
        installed: Installed,
        workspace: Entity<Workspace>,
        session: Option<SessionContext>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = PaneHost {
            installed: installed.clone(),
            workspace: workspace.downgrade(),
            window: window.window_handle(),
        };
        let local = cx.new(|cx| LocalPane::new(&installed, host.clone(), window, cx));
        let remote =
            cx.new(|cx| RemotePane::new(installed.local().clone(), host, session, window, cx));
        let local_events = cx.subscribe(&local, |this, _, event, cx| this.intent(event, cx));
        let remote_events = cx.subscribe(&remote, |this, _, event, cx| this.intent(event, cx));
        let workspace_events = cx.subscribe(&workspace, |this, workspace, event, cx| {
            if *event == WorkspaceEvent::ActiveSessionChanged {
                let workspace = workspace.clone();
                let panel = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let session = workspace.read(cx).active_session(cx);
                    let _ = panel.update(cx, |this, cx| {
                        this.remote
                            .update(cx, |remote, cx| remote.follow(session, cx))
                    });
                });
            } else if *event == WorkspaceEvent::LocalDirectoryChanged {
                this.local.update(cx, |_, cx| cx.notify());
            }
        });
        let tick = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            focus: cx.focus_handle(),
            local,
            remote,
            activity: activity::Activity::default(),
            divider: cx.new(|_| ResizableState::default()),
            collisions: CollisionPolicy::Skip,
            window: window.window_handle(),
            _observers: PanelObservers {
                _subscriptions: vec![local_events, remote_events, workspace_events],
                _tick: tick,
            },
            installed,
        }
    }
    fn intent(&mut self, event: &PaneIntent, cx: &mut Context<Self>) {
        match event {
            PaneIntent::Changed => self.synchronize(&[], cx),
            PaneIntent::Collision(policy) => {
                self.collisions = *policy;
                self.synchronize(&[], cx);
            }
            PaneIntent::Upload {
                sources,
                destination,
            } => {
                let result = self
                    .remote
                    .read(cx)
                    .transfer_target(destination.clone())
                    .map(|(fs, target, destination)| {
                        self.installed.transfers().enqueue(UploadRequest {
                            sources: sources.clone(),
                            target,
                            destination,
                            fs,
                            collisions: self.collisions,
                        })
                    })
                    .transpose();
                self.report_queue(
                    result.map(|v| v.is_some()),
                    "Connect to a server and open a remote directory before uploading.",
                    cx,
                );
            }
            PaneIntent::Download { files, destination } => {
                let destination = destination.clone().or_else(|| {
                    let local = self.local.read(cx);
                    local
                        .ready()
                        .then(|| local.directory().map(ToOwned::to_owned))
                        .flatten()
                });
                let result = destination
                    .map(|local_destination| {
                        self.installed
                            .transfers()
                            .enqueue_download(DownloadRequest {
                                sources: files.sources.clone(),
                                target: files.target.clone(),
                                local_destination,
                                fs: files.fs.clone(),
                                collisions: self.collisions,
                            })
                    })
                    .transpose();
                self.report_queue(
                    result.map(|v| v.is_some()),
                    "Open a local directory before downloading.",
                    cx,
                );
            }
        }
    }
    fn report_queue(
        &self,
        result: Result<bool, nocterm_transfers::TransferError>,
        unavailable: &'static str,
        cx: &mut Context<Self>,
    ) {
        let error = match result {
            Ok(true) => return,
            Ok(false) => unavailable.to_owned(),
            Err(error) => error.to_string(),
        };
        let _ = self.window.update(cx, |_, window, cx| {
            nocterm_ui::notice::error_action(
                window,
                cx,
                "files-enqueue",
                "Could not queue transfer",
                error,
                "Open Transfers",
                |window, cx| {
                    window.defer(cx, |window, cx| {
                        window.dispatch_action(Box::new(ShowTransfers), cx)
                    })
                },
            );
        });
    }
    fn synchronize(&mut self, finished: &[nocterm_transfers::Progress], cx: &mut Context<Self>) {
        let jobs = self.installed.transfers().snapshot();
        self.local.update(cx, |local, cx| {
            local.transfers(
                jobs.iter()
                    .filter(|job| {
                        job.direction == nocterm_transfers::TransferDirection::Download
                            && !job.state.finished()
                    })
                    .cloned()
                    .collect(),
                finished,
                cx,
            )
        });
        let ready = self.local.read(cx).ready();
        self.remote.update(cx, |remote, cx| {
            remote.transfers(jobs, finished, ready, self.collisions, cx)
        });
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (_, finished) = self.activity.update(self.installed.transfers().snapshot());
        self.synchronize(&finished, cx);
        self.local
            .update(cx, |local, cx| local.poll_statistics(window, cx));
        self.remote
            .update(cx, |remote, cx| remote.poll_statistics(cx));
    }
}
/// Current published indexing policy from this installation.
fn indexing(installed: &Installed, cx: &App) -> crate::IndexingSettings {
    installed
        .settings()
        .setting::<crate::ExplorerSettings>(cx)
        .indexing
        .clone()
}
impl Focusable for FilesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Panel for FilesPanel {
    fn title(&self, _: &App) -> SharedString {
        "Explorer".into()
    }
    fn icon(&self, _: &App) -> IconName {
        IconName::FolderTree
    }
}
impl Render for FilesPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus).min_h_0().child(
            v_resizable("explorer-divider")
                .with_state(&self.divider)
                .child(
                    resizable_panel()
                        .size_range(px(120.)..px(4000.))
                        .child(self.remote.clone()),
                )
                .child(
                    resizable_panel()
                        .size_range(px(140.)..px(4000.))
                        .child(self.local.clone()),
                ),
        )
    }
}
