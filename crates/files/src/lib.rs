//! A split Explorer with independent local navigation and pinned remote uploads.
mod dialogs;
mod local;
mod local_operations;
mod operations;
mod remote;
#[cfg(test)]
mod tests;
mod transfers;

use gpui_kit::{
    AnyElement, App, Context, Entity, ExternalPaths, FocusHandle, Focusable, MouseButton,
    SharedString, Subscription, Task, WeakEntity, Window,
    component::{
        ActiveTheme as _, Disableable as _, Icon, ResizableState, Selectable as _, Sizable as _,
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
use parking_lot::Mutex;
use remote::{Browser, listing};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

gpui_kit::actions!(
    files,
    [
        /// Open transfers and their progress, errors and cancellation controls.
        ShowTransfers,
    ]
);

pub fn register(workspace: &mut Workspace, cx: &mut Context<Workspace>) {
    transfers::init(cx);
    let active = workspace.active_session(cx);
    let handle = cx.entity();
    let panel = cx.new(|cx| FilesPanel::new(handle, active, cx));
    workspace.add_panel(panel, cx);
    let status = cx.new(transfers::TransferStatus::new);
    workspace.add_status_view(status, cx);
    workspace.register_action(|workspace, _: &ShowTransfers, window, cx| {
        if let Some(view) = workspace.find_item::<transfers::TransfersView>() {
            workspace.activate_item_by_id(view.entity_id(), window, cx);
        } else {
            let handle = cx.entity().downgrade();
            let view = cx.new(|cx| transfers::TransfersView::new(handle, cx));
            workspace.add_item(view, window, cx);
        }
    });
}

#[derive(Clone)]
struct LocalPaths(Vec<PathBuf>);
#[derive(Clone)]
struct RemotePaths {
    sources: Vec<String>,
    target: nocterm_session::Target,
    fs: Arc<dyn RemoteFs>,
}
struct DragPreview {
    count: usize,
    name: String,
}
impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .child(Icon::new(IconName::File).small())
            .child(if self.count == 1 {
                self.name.clone()
            } else {
                format!("{} items", self.count)
            })
    }
}
struct LocalBrowser {
    generation: u64,
    path: Option<PathBuf>,
    requested: Option<PathBuf>,
    entries: Vec<local::LocalEntry>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    loading: bool,
    error: Option<String>,
    progress: Arc<Mutex<local::Statistics>>,
    statistics: local::Statistics,
    cancel: Arc<AtomicBool>,
}
impl Default for LocalBrowser {
    fn default() -> Self {
        Self {
            generation: 0,
            path: None,
            requested: None,
            entries: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            loading: false,
            error: None,
            progress: Arc::default(),
            statistics: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
pub struct FilesPanel {
    focus: FocusHandle,
    session: Option<SessionContext>,
    browser: Browser,
    requested_directory: Option<String>,
    remote_selected: BTreeSet<usize>,
    remote_anchor: Option<usize>,
    local: LocalBrowser,
    workspace: WeakEntity<Workspace>,
    divider: Entity<ResizableState>,
    collisions: CollisionPolicy,
    upload_error: Option<String>,
    _subscription: Subscription,
    _tick: Task<()>,
    remote_task: Option<Task<()>>,
    local_task: Option<Task<()>>,
}
impl FilesPanel {
    fn remote_file_target(&self, index: usize) -> Option<FileTarget> {
        let entry = self.browser.entries.get(index)?;
        let session = self.session.as_ref()?;
        Some(FileTarget::Remote {
            path: path::join(self.browser.path.as_deref()?, &entry.name),
            host: session.target.clone(),
            fs: session.fs.clone()?,
        })
    }
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
    fn new(
        workspace: Entity<Workspace>,
        session: Option<SessionContext>,
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
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let statistics = this.local.progress.lock().clone();
                        if statistics != this.local.statistics {
                            this.local.statistics = statistics;
                            cx.notify();
                        }
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
            workspace: workspace.downgrade(),
            divider: cx.new(|_| ResizableState::default()),
            collisions: CollisionPolicy::Skip,
            upload_error: None,
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
    fn load(&mut self, directory: Option<String>, cx: &mut Context<Self>) {
        let Some(fs) = self.filesystem() else {
            return;
        };
        let generation = self.browser.begin();
        self.remote_selected.clear();
        self.remote_anchor = None;
        self.requested_directory = directory.clone();
        cx.notify();
        self.remote_task = Some(cx.spawn(async move |this, cx| {
            let result = listing(fs, directory).await;
            let _ = this.update(cx, |this, cx| {
                if this.browser.finish(generation, result) {
                    cx.notify();
                }
            });
        }));
    }
    fn load_local(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.local.cancel.store(true, Ordering::Release);
        self.local.cancel = Arc::new(AtomicBool::new(false));
        self.local.generation = self.local.generation.wrapping_add(1);
        let generation = self.local.generation;
        self.local.loading = true;
        self.local.error = None;
        self.local.requested = Some(directory.clone());
        let cancel = self.local.cancel.clone();
        let progress = Arc::new(Mutex::new(local::Statistics::default()));
        self.local.progress = progress.clone();
        self.local.statistics = Default::default();
        cx.notify();
        self.local_task = Some(cx.spawn(async move |this, cx| {
            let path = directory.clone();
            let result = cx
                .background_executor()
                .spawn(async move { local::read_directory(&path) })
                .await;
            let loaded = this
                .update(cx, |this, cx| {
                    if this.local.generation != generation {
                        return false;
                    }
                    this.local.loading = false;
                    match result {
                        Ok(entries) => {
                            this.local.path = Some(directory.clone());
                            this.local.entries = entries;
                            this.local.selected.clear();
                            this.local.anchor = None;
                        }
                        Err(error) => this.local.error = Some(error),
                    }
                    cx.notify();
                    this.local.error.is_none()
                })
                .unwrap_or(false);
            if loaded {
                cx.background_executor()
                    .spawn(async move {
                        local::scan(directory, cancel, progress);
                    })
                    .await;
            }
        }));
    }
    fn enqueue(
        &mut self,
        sources: Vec<PathBuf>,
        destination: Option<String>,
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
        self.upload_error = result.err();
        cx.notify();
    }
    fn enqueue_download(
        &mut self,
        files: RemotePaths,
        destination: Option<PathBuf>,
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
        self.upload_error = result.err();
        cx.notify();
    }
    fn remote_paths(&self, indices: impl Iterator<Item = usize>) -> Option<RemotePaths> {
        let parent = self.browser.path.as_deref()?;
        let sources = indices
            .filter_map(|index| self.browser.entries.get(index))
            .map(|entry| path::join(parent, &entry.name))
            .collect();
        Some(RemotePaths {
            sources,
            target: self.session.as_ref()?.target.clone(),
            fs: self.filesystem()?,
        })
    }
    fn select_remote(
        &mut self,
        ix: usize,
        event: &gpui_kit::MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.shift {
            let anchor = self.remote_anchor.unwrap_or(ix);
            self.remote_selected.extend(anchor.min(ix)..=anchor.max(ix));
        } else if event.modifiers.control || event.modifiers.platform {
            if !self.remote_selected.insert(ix) {
                self.remote_selected.remove(&ix);
            }
            self.remote_anchor = Some(ix);
        } else {
            self.remote_selected.clear();
            self.remote_selected.insert(ix);
            self.remote_anchor = Some(ix);
        }
        if event.click_count == 2
            && let Some(entry) = self
                .browser
                .entries
                .get(ix)
                .filter(|entry| entry.kind == EntryKind::Directory)
        {
            let destination = self
                .browser
                .path
                .as_deref()
                .map(|parent| path::join(parent, &entry.name));
            self.load(destination, cx);
        } else {
            cx.notify();
        }
    }
    fn select_local(
        &mut self,
        ix: usize,
        event: &gpui_kit::MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.shift {
            let anchor = self.local.anchor.unwrap_or(ix);
            self.local.selected.extend(anchor.min(ix)..=anchor.max(ix));
        } else if event.modifiers.control || event.modifiers.platform {
            if !self.local.selected.insert(ix) {
                self.local.selected.remove(&ix);
            }
            self.local.anchor = Some(ix);
        } else {
            self.local.selected.clear();
            self.local.selected.insert(ix);
            self.local.anchor = Some(ix);
        }
        if event.click_count == 2
            && let Some(entry) = self
                .local
                .entries
                .get(ix)
                .filter(|e| e.kind == EntryKind::Directory && !e.symlink)
        {
            self.load_local(entry.path.clone(), cx);
        } else {
            cx.notify();
        }
    }
    fn local_paths(&self, index: usize) -> Vec<PathBuf> {
        if self.local.selected.contains(&index) {
            self.local
                .selected
                .iter()
                .filter_map(|ix| self.local.entries.get(*ix))
                .map(|e| e.path.clone())
                .collect()
        } else {
            self.local
                .entries
                .get(index)
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default()
        }
    }
    fn render_remote_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.browser.entries[ix];
        let directory = entry.kind == EntryKind::Directory;
        let selected: Vec<usize> = if self.remote_selected.contains(&ix) {
            self.remote_selected.iter().copied().collect()
        } else {
            vec![ix]
        };
        let download = self.remote_paths(selected.iter().copied());
        let preview = entry.name.clone();
        let destination = self
            .browser
            .path
            .as_deref()
            .map(|parent| path::join(parent, &entry.name));
        let drop_path = destination;
        let enabled = self.filesystem().is_some() && !self.browser.loading;
        let target = self.remote_file_target(ix);
        let selected_targets = selected
            .into_iter()
            .filter_map(|ix| self.remote_file_target(ix))
            .collect::<Vec<_>>();
        let refresh = self.refresh_after_mutation(false, cx);
        div()
            .id(("remote-entry", ix))
            .w_full()
            .h_8()
            .text_size(px(cx.design().typography.explorer_size.unwrap_or(12.0)))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_sm()
            .when(enabled, |row| row.cursor_pointer())
            .when(self.remote_selected.contains(&ix), |row| {
                row.bg(cx.theme().accent)
            })
            .hover(|s| s.bg(cx.theme().accent))
            .child(
                Icon::new(if directory {
                    IconName::Folder
                } else {
                    IconName::File
                })
                .small(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!(
                        "{}{}",
                        entry.name,
                        if entry.is_symlink { " (link)" } else { "" }
                    )),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, _, cx| this.select_remote(ix, event, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    if !this.remote_selected.contains(&ix) {
                        this.remote_selected.clear();
                        this.remote_selected.insert(ix);
                        this.remote_anchor = Some(ix);
                        cx.notify();
                    }
                }),
            )
            .when_some(download, |row, download| {
                row.on_drag(download, move |files, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| DragPreview {
                        count: files.sources.len(),
                        name: preview.clone(),
                    })
                })
            })
            .when(directory && enabled, |row| {
                row.drag_over::<LocalPaths>(|s, _, _, cx| {
                    s.bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().primary)
                })
                .drag_over::<ExternalPaths>(|s, _, _, cx| {
                    s.bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().primary)
                })
                .on_drop(cx.listener({
                    let drop_path = drop_path.clone();
                    move |this, files: &LocalPaths, _, cx| {
                        cx.stop_propagation();
                        this.enqueue(files.0.clone(), drop_path.clone(), cx);
                    }
                }))
                .on_drop(cx.listener(
                    move |this, files: &ExternalPaths, _, cx| {
                        cx.stop_propagation();
                        this.enqueue(files.paths().to_vec(), drop_path.clone(), cx);
                    },
                ))
            })
            .context_menu(move |menu, _, _| match &target {
                Some(target) => dialogs::menu(
                    menu,
                    target.clone(),
                    selected_targets.clone(),
                    refresh.clone(),
                    enabled,
                ),
                None => menu,
            })
            .into_any_element()
    }
    fn render_remote(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.filesystem().is_some() && !self.browser.loading;
        let hint = match &self.session {
            None => Some("Open a connection to browse remote files."),
            Some(s) if !s.connected => Some("Disconnected. Reconnect to browse files."),
            Some(s) if s.fs.is_none() => Some("This host does not provide SFTP."),
            _ => None,
        };
        v_flex()
            .id("remote-browser")
            .size_full()
            .min_h_0()
            .gap_1()
            .px_2()
            .pb_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(div().flex_1().text_sm().child("Remote"))
                    .child(
                        Button::new("files-download")
                            .small()
                            .ghost()
                            .icon(IconName::ArrowDown)
                            .tooltip("Download selected items to the local Explorer directory")
                            .disabled(
                                !enabled
                                    || self.remote_selected.is_empty()
                                    || self.local.loading
                                    || self.local.path.is_none(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(files) =
                                    this.remote_paths(this.remote_selected.iter().copied())
                                {
                                    this.enqueue_download(files, None, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("files-home")
                            .small()
                            .ghost()
                            .label("Home")
                            .disabled(!enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.load(None, cx))),
                    )
                    .child(
                        Button::new("files-parent")
                            .small()
                            .ghost()
                            .label("Up")
                            .disabled(
                                !enabled || self.browser.path.as_deref().is_none_or(|p| p == "/"),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load(
                                    this.browser
                                        .path
                                        .as_deref()
                                        .map(|p| path::parent(p).to_owned()),
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("files-refresh")
                            .small()
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh remote directory")
                            .disabled(!enabled)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load(this.requested_directory.clone(), cx)
                            })),
                    ),
            )
            .when_some(self.browser.path.clone(), |p, path| {
                p.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .text_ellipsis()
                        .overflow_hidden()
                        .child(path),
                )
            })
            .when_some(hint, |p, hint| {
                p.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                )
            })
            .when(self.browser.loading, |p| {
                p.child(div().text_sm().child("Loading directory…"))
            })
            .when_some(self.browser.error.as_ref(), |p, e| {
                p.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(format!("{e}. Refresh to retry.")),
                )
            })
            .when(
                enabled && self.browser.entries.is_empty() && self.browser.error.is_none(),
                |p| {
                    p.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Empty directory. Drop local files here to upload."),
                    )
                },
            )
            .child(
                uniform_list(
                    "remote-file-list",
                    self.browser.entries.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| this.render_remote_row(ix, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Existing:"),
                    )
                    .children(
                        [
                            ("skip", "Skip", CollisionPolicy::Skip),
                            ("rename", "Rename", CollisionPolicy::Rename),
                            ("replace", "Replace", CollisionPolicy::Replace),
                        ]
                        .into_iter()
                        .map(|(id, label, policy)| {
                            Button::new(SharedString::from(format!("collision-policy-{id}")))
                                .xsmall()
                                .ghost()
                                .label(label)
                                .selected(self.collisions == policy)
                                .tooltip(match policy {
                                    CollisionPolicy::Skip => "Skip existing names",
                                    CollisionPolicy::Rename => "Choose a new filename",
                                    CollisionPolicy::Replace => {
                                        "Replace existing files atomically if supported"
                                    }
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.collisions = policy;
                                    cx.notify();
                                }))
                        }),
                    ),
            )
            .when_some(self.upload_error.as_ref(), |p, e| {
                p.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(e.clone()),
                )
            })
            .drag_over::<LocalPaths>(|s, _, _, cx| {
                s.bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().primary)
            })
            .drag_over::<ExternalPaths>(|s, _, _, cx| {
                s.bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().primary)
            })
            .on_drop(cx.listener(|this, files: &LocalPaths, _, cx| {
                this.enqueue(files.0.clone(), None, cx)
            }))
            .on_drop(cx.listener(|this, files: &ExternalPaths, _, cx| {
                this.enqueue(files.paths().to_vec(), None, cx)
            }))
            .into_any_element()
    }
    fn render_local_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.local.entries[ix];
        let paths = self.local_paths(ix);
        let preview = entry.name.clone();
        let destination = entry.path.clone();
        let directory = entry.kind == EntryKind::Directory && !entry.symlink;
        let target = FileTarget::Local(entry.path.clone());
        let selected_targets = paths
            .iter()
            .cloned()
            .map(FileTarget::Local)
            .collect::<Vec<_>>();
        let refresh = self.refresh_after_mutation(true, cx);
        let enabled = !self.local.loading;
        div()
            .id(("local-entry", ix))
            .w_full()
            .h_8()
            .text_size(px(cx.design().typography.explorer_size.unwrap_or(12.0)))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_sm()
            .cursor_pointer()
            .when(self.local.selected.contains(&ix), |row| {
                row.bg(cx.theme().accent)
            })
            .hover(|s| s.bg(cx.theme().accent))
            .child(
                Icon::new(if entry.kind == EntryKind::Directory {
                    IconName::Folder
                } else {
                    IconName::File
                })
                .small(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!(
                        "{}{}",
                        entry.name,
                        if entry.symlink { " (link)" } else { "" }
                    )),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, _, cx| this.select_local(ix, event, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    if !this.local.selected.contains(&ix) {
                        this.local.selected.clear();
                        this.local.selected.insert(ix);
                        this.local.anchor = Some(ix);
                        cx.notify();
                    }
                }),
            )
            .on_drag(LocalPaths(paths), move |files, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| DragPreview {
                    count: files.0.len(),
                    name: preview.clone(),
                })
            })
            .when(directory, |row| {
                row.drag_over::<RemotePaths>(|style, _, _, cx| {
                    style
                        .bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().primary)
                })
                .on_drop(cx.listener(move |this, files: &RemotePaths, _, cx| {
                    cx.stop_propagation();
                    this.enqueue_download(files.clone(), Some(destination.clone()), cx);
                }))
            })
            .context_menu(move |menu, _, _| {
                dialogs::menu(
                    menu,
                    target.clone(),
                    selected_targets.clone(),
                    refresh.clone(),
                    enabled,
                )
            })
            .into_any_element()
    }
    fn render_local(&self, cx: &mut Context<Self>) -> AnyElement {
        let cwd = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).local_terminal_cwd(cx));
        let s = &self.local.statistics;
        let information = format!(
            "{} files · {} folders · {}{}{}",
            s.files,
            s.directories,
            local::bytes(s.bytes),
            if s.complete { "" } else { " · calculating…" },
            if s.inaccessible > 0 {
                " · incomplete"
            } else {
                ""
            }
        );
        v_flex()
            .id("local-browser")
            .size_full()
            .min_h_0()
            .gap_1()
            .px_2()
            .pt_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(div().flex_1().text_sm().child("Local"))
                    .child(
                        Button::new("local-home")
                            .small()
                            .ghost()
                            .label("Home")
                            .disabled(self.local.loading)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(home) = local::home() {
                                    this.load_local(home, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("local-parent")
                            .small()
                            .ghost()
                            .label("Up")
                            .disabled(
                                self.local.loading
                                    || self
                                        .local
                                        .path
                                        .as_deref()
                                        .and_then(|p| p.parent())
                                        .is_none(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(parent) = this
                                    .local
                                    .path
                                    .as_deref()
                                    .and_then(|p| p.parent())
                                    .map(|p| p.to_owned())
                                {
                                    this.load_local(parent, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("local-refresh")
                            .small()
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh local directory")
                            .disabled(self.local.loading)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(path) = this.local.requested.clone() {
                                    this.load_local(path, cx);
                                }
                            })),
                    ),
            )
            .when_some(self.local.path.as_ref(), |p, path| {
                p.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .text_ellipsis()
                        .overflow_hidden()
                        .child(path.display().to_string()),
                )
            })
            .when(self.local.loading, |p| {
                p.child(div().text_sm().child("Loading directory…"))
            })
            .when_some(self.local.error.as_ref(), |p, e| {
                p.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(e.clone()),
                )
            })
            .when(
                !self.local.loading && self.local.entries.is_empty() && self.local.error.is_none(),
                |p| {
                    p.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("This directory is empty."),
                    )
                },
            )
            .child(
                uniform_list(
                    "local-file-list",
                    self.local.entries.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| this.render_local_row(ix, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .child(
                v_flex()
                    .gap_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .py_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(information),
                    )
                    .when(s.inaccessible > 0, |p| {
                        p.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(format!("{} paths could not be scanned", s.inaccessible)),
                        )
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new("explorer-to-terminal")
                                    .small()
                                    .ghost()
                                    .icon(IconName::ArrowDown)
                                    .tooltip("Explorer → Local terminal")
                                    .disabled(self.local.path.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let (Some(path), Some(workspace)) =
                                            (this.local.path.clone(), this.workspace.upgrade())
                                        {
                                            let result = workspace.update(cx, |workspace, cx| {
                                                workspace.change_local_directory(path, window, cx)
                                            });
                                            this.local.error = result.err();
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Button::new("terminal-to-explorer")
                                    .small()
                                    .ghost()
                                    .icon(IconName::ArrowUp)
                                    .tooltip("Local terminal → Explorer")
                                    .disabled(cwd.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(path) =
                                            this.workspace.upgrade().and_then(|workspace| {
                                                workspace.read(cx).local_terminal_cwd(cx)
                                            })
                                        {
                                            this.load_local(path, cx);
                                        }
                                    })),
                            ),
                    ),
            )
            .drag_over::<RemotePaths>(|style, _, _, cx| {
                style
                    .bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().primary)
            })
            .on_drop(cx.listener(|this, files: &RemotePaths, _, cx| {
                cx.stop_propagation();
                this.enqueue_download(files.clone(), None, cx);
            }))
            .into_any_element()
    }
}
impl Drop for FilesPanel {
    fn drop(&mut self) {
        self.local.cancel.store(true, Ordering::Release);
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
