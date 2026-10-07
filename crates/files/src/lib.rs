//! A split Explorer with independent local navigation and pinned remote uploads.
mod activity;
mod dialogs;
mod local;
mod local_operations;
mod local_pane;
mod open;
mod operations;
mod registration;
mod remote;
mod remote_pane;
mod row;
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
use nocterm_ui::{ActiveDesign as _, ActiveSettings as _, IconName};
use nocterm_workspace::{Panel, SessionContext, Workspace, WorkspaceEvent};
use operations::FileTarget;
use remote::{Browser, listing};
use std::{collections::BTreeSet, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

pub use registration::{OpenExplorerSettings, ShowTransfers, ToggleExplorer, register};
pub use settings_page::ExplorerPage;

/// Supplies the lazy Explorer page to the application Settings host.
pub fn settings_page() -> nocterm_workspace::SettingsPageSpec {
    nocterm_workspace::SettingsPageSpec::new("explorer", "Explorer", |window, cx| {
        cx.new(|cx| ExplorerPage::new(window, cx))
    })
    .with_icon(IconName::FolderTree)
}

#[derive(Clone)]
struct LocalPaths(Vec<PathBuf>);
#[derive(Clone)]
struct RemotePaths {
    sources: Vec<String>,
    target: nocterm_session::Target,
    fs: Arc<dyn RemoteFs>,
}
#[derive(Default)]
struct LocalBrowser {
    generation: u64,
    path: Option<PathBuf>,
    requested: Option<PathBuf>,
    entries: Vec<local::LocalEntry>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    loading: bool,
    error: Option<String>,
    counter: statistics::Counter,
}
pub struct FilesPanel {
    focus: FocusHandle,
    session: Option<SessionContext>,
    browser: Browser,
    requested_directory: Option<String>,
    remote_selected: BTreeSet<usize>,
    remote_anchor: Option<usize>,
    local: LocalBrowser,
    remote_counter: statistics::Counter,
    activity: activity::Activity,
    /// A finished transfer changed the shown folder: (remote, local).
    stale: (bool, bool),
    workspace: WeakEntity<Workspace>,
    divider: Entity<ResizableState>,
    collisions: CollisionPolicy,
    statistics_reported: bool,
    window: AnyWindowHandle,
    _subscription: Subscription,
    _tick: Task<()>,
    remote_task: Option<Task<()>>,
    local_task: Option<Task<()>>,
}
impl FilesPanel {
    fn refresh_after_mutation(&self, local: bool, cx: &Context<Self>) -> dialogs::Refresh {
        let panel = cx.entity().downgrade();
        let generation = if local {
            self.local.generation
        } else {
            self.browser.generation
        };
        let filesystem = self.session.as_ref().and_then(|s| s.fs.clone());
        Rc::new(move |cx| {
            let _ = panel.update(cx, |this, cx| {
                if local {
                    if this.local.generation == generation
                        && let Some(path) = this.local.path.clone()
                    {
                        this.load_local(path, cx);
                    }
                } else if this.browser.generation == generation
                    && filesystem
                        .as_ref()
                        .zip(this.filesystem().as_ref())
                        .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
                {
                    this.load(this.browser.path.clone(), cx);
                }
            });
        })
    }
    /// Follows the transfer queue and lists again a shown folder that a
    /// finished transfer changed.
    fn poll_transfers(&mut self, cx: &mut Context<Self>) {
        let snapshot = transfers::service(cx)
            .map(|service| service.snapshot())
            .unwrap_or_default();
        let (changed, finished) = self.activity.update(snapshot);
        if changed {
            cx.notify();
        }
        let remote = self
            .session
            .as_ref()
            .map(|s| s.target.clone())
            .zip(self.browser.path.clone());
        if let Some((target, shown)) = &remote
            && finished
                .iter()
                .any(|job| activity::changes_remote(job, target, shown))
        {
            self.stale.0 = true;
        }
        if let Some(shown) = &self.local.path
            && finished
                .iter()
                .any(|job| activity::changes_local(job, shown))
        {
            self.stale.1 = true;
        }
        // A folder still loading is listed again once that load is over.
        if self.stale.0 && !self.browser.loading {
            self.stale.0 = false;
            self.load(self.requested_directory.clone(), cx);
        }
        if self.stale.1
            && !self.local.loading
            && let Some(shown) = self.local.path.clone()
        {
            self.stale.1 = false;
            self.load_local(shown, cx);
        }
    }
    /// Shows the walks' latest counts and reports a partial local count once.
    fn poll_statistics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_counter.poll() {
            cx.notify();
        }
        if !self.local.counter.poll() {
            return;
        }
        cx.notify();
        let statistics = &self.local.counter.shown;
        if !statistics.complete || statistics.inaccessible == 0 || self.statistics_reported {
            return;
        }
        self.statistics_reported = true;
        let Some(path) = self.local.path.clone() else {
            return;
        };
        let message = statistics.errors.first().cloned().unwrap_or_else(|| {
            format!(
                "{} paths could not be scanned; folder size is partial.",
                statistics.inaccessible
            )
        });
        let generation = self.local.generation;
        let panel = cx.entity().downgrade();
        nocterm_ui::notice::warning_action(
            window,
            cx,
            "files-partial-statistics",
            "Folder size is partial",
            message,
            "Recalculate",
            move |_, cx| {
                let _ = panel.update(cx, |this, cx| {
                    if this.local.generation == generation
                        && this.local.path.as_ref() == Some(&path)
                    {
                        this.load_local(path.clone(), cx);
                    }
                });
            },
        );
    }
    fn new(
        workspace: Entity<Workspace>,
        session: Option<SessionContext>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&workspace, |_, workspace, event, cx| {
            if *event == WorkspaceEvent::ActiveSessionChanged {
                let workspace = workspace.clone();
                let panel = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let session = workspace.read(cx).active_session(cx);
                    let _ = panel.update(cx, |this, cx| this.follow(session, cx));
                });
            } else if *event == WorkspaceEvent::LocalDirectoryChanged {
                cx.notify();
            }
        });
        let tick = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        this.poll_transfers(cx);
                        this.poll_statistics(window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut panel = Self {
            focus: cx.focus_handle(),
            session: None,
            browser: Browser::default(),
            requested_directory: None,
            remote_selected: BTreeSet::new(),
            remote_anchor: None,
            local: LocalBrowser::default(),
            remote_counter: statistics::Counter::default(),
            activity: activity::Activity::default(),
            stale: (false, false),
            workspace: workspace.downgrade(),
            divider: cx.new(|_| ResizableState::default()),
            collisions: CollisionPolicy::Skip,
            statistics_reported: false,
            window: window.window_handle(),
            _subscription: subscription,
            _tick: tick,
            remote_task: None,
            local_task: None,
        };
        panel.follow(session, cx);
        panel
    }
    fn follow(&mut self, session: Option<SessionContext>, cx: &mut Context<Self>) {
        if matches!((&self.session, &session), (Some(before), Some(after)) if before.same_as(after))
        {
            return;
        }
        self.session = session;
        self.requested_directory = None;
        self.browser.clear();
        self.remote_selected.clear();
        self.remote_anchor = None;
        self.remote_task = None;
        self.remote_counter.set(Default::default());
        if self.filesystem().is_some() {
            self.load(None, cx);
        }
        cx.notify();
    }
    fn filesystem(&self) -> Option<Arc<dyn RemoteFs>> {
        self.session
            .as_ref()
            .filter(|s| s.connected)
            .and_then(|s| s.fs.clone())
    }
    fn enqueue(
        &mut self,
        sources: Vec<PathBuf>,
        destination: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match (
            self.filesystem(),
            self.session.as_ref(),
            destination.or_else(|| self.browser.path.clone()),
        ) {
            (Some(fs), Some(session), Some(destination)) => {
                transfers::service(cx).and_then(|service| {
                    service
                        .enqueue(UploadRequest {
                            sources,
                            target: session.target.clone(),
                            destination,
                            fs,
                            collisions: self.collisions,
                        })
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
            }
            _ => Err("Connect to a server and open a remote directory before uploading.".into()),
        };
        if let Err(error) = result {
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
                    });
                },
            );
        }
        cx.notify();
    }
    fn enqueue_download(
        &mut self,
        files: RemotePaths,
        destination: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = destination
            .or_else(|| self.local.path.clone())
            .ok_or_else(|| "Open a local directory before downloading.".to_owned())
            .and_then(|local_destination| {
                transfers::service(cx).and_then(|service| {
                    service
                        .enqueue_download(DownloadRequest {
                            sources: files.sources,
                            target: files.target,
                            local_destination,
                            fs: files.fs,
                            collisions: self.collisions,
                        })
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                })
            });
        if let Err(error) = result {
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
                    });
                },
            );
        }
        cx.notify();
    }
}
/// The user's indexing settings; the defaults where none are installed.
fn indexing(cx: &App) -> nocterm_settings::IndexingSettings {
    if cx.has_global::<nocterm_ui::SettingsStore>() {
        cx.settings().explorer.indexing.clone()
    } else {
        Default::default()
    }
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.local.requested.is_none() {
            let initial = cx
                .has_global::<nocterm_ui::SettingsStore>()
                .then(|| cx.settings().local.cwd.clone())
                .flatten()
                .map(PathBuf::from)
                .or_else(local::home);
            if let Some(path) = initial {
                self.load_local(path, cx);
            }
        }
        div().size_full().track_focus(&self.focus).min_h_0().child(
            v_resizable("explorer-divider")
                .with_state(&self.divider)
                .child(
                    resizable_panel()
                        .size_range(px(120.)..px(4000.))
                        .child(self.render_remote(cx)),
                )
                .child(
                    resizable_panel()
                        .size_range(px(140.)..px(4000.))
                        .child(self.render_local(cx)),
                ),
        )
    }
}
