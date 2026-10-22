//! The Explorer's remote half: browsing the active session's host.
use super::*;
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use row::ExplorerRow;

use nocterm_transfers::Progress;
struct RemotePresentation {
    host: PaneHost,
    activity: Vec<Progress>,
    collisions: CollisionPolicy,
    download_enabled: bool,
}
impl RemotePresentation {
    fn hint(source: Option<&ExplorerSource>) -> Option<&'static str> {
        match source {
            None => Some("Open a connection to browse remote files."),
            Some(source) if source.is_local() => Some("Open a connection to browse remote files."),
            Some(source) if !source.connected() => Some("Disconnected. Reconnect to browse files."),
            Some(source) if source.filesystem().is_none() => {
                Some("This host does not provide SFTP.")
            }
            _ => None,
        }
    }
}
pub(super) struct RemotePane {
    source: Option<nocterm_workspace::ExplorerSource>,
    browser: Browser,
    requested: Option<String>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    path: Entity<path_input::PathInput>,
    _subscription: Subscription,
    counter: statistics::Counter,
    task: Option<Task<()>>,
    stale: bool,
    presentation: RemotePresentation,
}
impl EventEmitter<PaneIntent> for RemotePane {}
impl RemotePane {
    pub(super) fn new(
        local: Arc<dyn nocterm_session::local_fs::LocalFs>,
        host: PaneHost,
        source: Option<nocterm_workspace::ExplorerSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let path = cx.new(|cx| path_input::PathInput::new(local, window, cx));
        let subscription = cx.subscribe(&path, |this, _, event: &path_input::Navigate, cx| {
            if let path_input::Directory::Remote(path) = &event.directory {
                this.load_with_navigation(Some(path.clone()), event.mode, cx);
            }
        });
        let mut pane = Self {
            source: None,
            browser: Browser::default(),
            requested: None,
            selected: BTreeSet::new(),
            anchor: None,
            path,
            _subscription: subscription,
            counter: statistics::Counter::default(),
            task: None,
            stale: false,
            presentation: RemotePresentation {
                host,
                activity: Vec::new(),
                collisions: CollisionPolicy::Skip,
                download_enabled: false,
            },
        };
        let _ = pane.follow(source, cx);
        pane
    }
    pub(super) fn listing(&self) -> ListingState<String> {
        ListingState {
            path: self.browser.path().map(str::to_owned),
            requested: self.requested.clone(),
            names: self
                .browser
                .entries()
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
            selected: self.selected.iter().copied().collect(),
            loading: self.browser.loading(),
            error: self.browser.error().map(ToString::to_string),
            statistics: statistics::summary(self.counter.shown()),
        }
    }
    pub(super) fn access(&self) -> RemoteAccess {
        match &self.source {
            None => RemoteAccess::NoSession,
            Some(session) if session.is_local() => RemoteAccess::NoSession,
            Some(session) if !session.connected() => RemoteAccess::Disconnected,
            Some(session) if session.filesystem().is_none() => RemoteAccess::Unsupported,
            Some(_) => RemoteAccess::Available,
        }
    }
    pub(super) fn transfers(
        &mut self,
        jobs: Vec<Progress>,
        finished: &[Progress],
        download_enabled: bool,
        collisions: CollisionPolicy,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = &self.source
            && let Some(shown) = self.browser.path()
            && finished.iter().any(|job| {
                session
                    .address()
                    .is_some_and(|address| activity::changes_remote(job, &address, shown))
            })
        {
            self.stale = true;
        }
        let activity = self
            .source
            .as_ref()
            .map(|session| {
                jobs.into_iter()
                    .filter(|job| {
                        Some(&job.host) == session.address().as_ref()
                            && job.direction == nocterm_transfers::TransferDirection::Upload
                            && !job.state.finished()
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self.presentation.activity != activity
            || self.presentation.download_enabled != download_enabled
            || self.presentation.collisions != collisions
        {
            self.presentation.activity = activity;
            self.presentation.download_enabled = download_enabled;
            self.presentation.collisions = collisions;
            cx.notify();
        }
        if self.stale && !self.browser.loading() {
            self.stale = false;
            self.load(self.requested.clone(), cx);
        }
    }
    pub(super) fn poll_statistics(&mut self, cx: &mut Context<Self>) {
        if self.counter.poll() {
            cx.notify();
        }
    }
    fn refresh_after_mutation(&self, cx: &Context<Self>) -> dialogs::Refresh {
        let pane = cx.entity().downgrade();
        let generation = self.browser.generation();
        let fs = self.filesystem();
        Rc::new(move |cx| {
            let _ = pane.update(cx, |this, cx| {
                if this.browser.is_current(generation)
                    && fs
                        .as_ref()
                        .zip(this.filesystem().as_ref())
                        .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
                {
                    this.load(this.browser.path().map(str::to_owned), cx);
                }
            });
        })
    }
    pub(super) fn follow(
        &mut self,
        source: Option<nocterm_workspace::ExplorerSource>,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        if matches!((&self.source, &source), (Some(before), Some(after)) if before.same_as(after)) {
            return None;
        }
        let initial_directory = source
            .as_ref()
            .and_then(ExplorerSource::initial_directory)
            .map(str::to_owned);
        self.source = source;
        let filesystem = self.filesystem();
        self.path
            .update(cx, |input, cx| input.reset_remote(filesystem, cx));
        self.requested = None;
        self.browser.clear();
        self.selected.clear();
        self.anchor = None;
        self.task = None;
        self.counter.set(Default::default());
        if self.filesystem().is_some() {
            self.load(initial_directory, cx);
        }
        cx.notify();
        self.source
            .as_ref()
            .filter(|source| source.is_local())
            .and_then(ExplorerSource::initial_directory)
            .map(PathBuf::from)
    }
    pub(super) fn filesystem(&self) -> Option<Arc<dyn RemoteFs>> {
        self.source
            .as_ref()
            .filter(|s| s.connected() && !s.is_local())
            .and_then(|s| s.filesystem())
    }
}
impl RemotePane {
    pub(super) fn load(&mut self, directory: Option<String>, cx: &mut Context<Self>) {
        self.load_with_navigation(directory, path_input::NavigationMode::CloseEditor, cx);
    }

    pub(super) fn load_with_navigation(
        &mut self,
        directory: Option<String>,
        mode: path_input::NavigationMode,
        cx: &mut Context<Self>,
    ) {
        let Some(fs) = self.filesystem() else {
            return;
        };
        self.path
            .update(cx, |input, cx| input.begin_navigation_with_mode(mode, cx));
        let generation = self.browser.begin();
        self.selected.clear();
        self.anchor = None;
        self.requested = directory.clone();
        cx.notify();
        let window = self.presentation.host.window;
        let retry_directory = directory.clone();
        let home_requested = directory.is_none();
        self.task = Some(cx.spawn(async move |this, cx| {
            let listing_fs = fs.clone();
            let result = cx
                .background_executor()
                .spawn(async move { listing(listing_fs, directory).await })
                .await;
            let error = this
                .update(cx, |this, cx| {
                    if this.browser.finish(generation, result) {
                        if this.browser.error().is_none() {
                            if home_requested {
                                this.browser.remember_current_path_as_home();
                            }
                            if let Some(path) = this.browser.path().map(str::to_owned) {
                                let home = this
                                    .browser
                                    .home()
                                    .map(str::to_owned)
                                    .map(path_input::Directory::Remote);
                                this.path.update(cx, |input, cx| {
                                    input.accept(path_input::Directory::Remote(path), home, cx)
                                });
                            }
                            this.count_remote(fs, cx);
                        }
                        if let Some(error) = this.browser.error() {
                            let error = error.to_string();
                            this.path
                                .update(cx, |input, cx| input.navigation_failed(error, cx));
                        }
                        cx.notify();
                        this.browser.error().map(ToString::to_string)
                    } else {
                        None
                    }
                })
                .ok()
                .flatten();
            if let Some(error) = error {
                let panel = this.clone();
                let _ = window.update(cx, |_, window, cx| {
                    nocterm_ui::notice::error_action(
                        window,
                        cx,
                        "files-remote-list",
                        "Could not load remote directory",
                        error,
                        "Retry",
                        move |_, cx| {
                            let _ = panel.update(cx, |this, cx| {
                                if this.browser.is_current(generation) {
                                    this.load(retry_directory.clone(), cx);
                                }
                            });
                        },
                    );
                });
            }
        }));
    }

    /// Counts the shown remote folder, as the indexing settings allow.
    fn count_remote(&mut self, fs: Arc<dyn RemoteFs>, cx: &mut Context<Self>) {
        let Some(directory) = self.browser.path().map(str::to_owned) else {
            return;
        };
        let indexing = indexing(&self.presentation.host.installed, cx);
        let skip = statistics::remote_skip(&directory, self.browser.home(), &indexing);
        if let Some(skip) = skip {
            let kinds = self.browser.entries().iter().map(|entry| entry.kind);
            self.counter.set(statistics::shallow(kinds, skip));
            return;
        }
        let (cancel, progress) = self.counter.restart();
        let policy = statistics::Policy::remote(&indexing);
        cx.background_executor()
            .spawn(statistics::scan_remote(
                fs, directory, policy, cancel, progress,
            ))
            .detach();
    }

    pub(super) fn remote_file_target(&self, index: usize) -> Option<FileTarget> {
        let entry = self.browser.entries().get(index)?;
        let session = self.source.as_ref()?;
        Some(FileTarget::Remote {
            path: path::join(self.browser.path()?, &entry.name),
            host: session.address()?,
            fs: session.filesystem()?,
        })
    }

    pub(super) fn paths(&self, indices: impl Iterator<Item = usize>) -> Option<RemotePaths> {
        let parent = self.browser.path()?;
        let sources = indices
            .filter_map(|index| self.browser.entries().get(index))
            .map(|entry| path::join(parent, &entry.name))
            .collect();
        Some(RemotePaths {
            sources,
            host: self.source.as_ref()?.address()?,
            fs: self.filesystem()?,
        })
    }

    pub(super) fn select_remote(
        &mut self,
        ix: usize,
        event: &gpui_kit::MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.shift {
            let anchor = self.anchor.unwrap_or(ix);
            self.selected.extend(anchor.min(ix)..=anchor.max(ix));
        } else if event.modifiers.control || event.modifiers.platform {
            if !self.selected.insert(ix) {
                self.selected.remove(&ix);
            }
            self.anchor = Some(ix);
        } else {
            self.selected.clear();
            self.selected.insert(ix);
            self.anchor = Some(ix);
        }
        if event.click_count == 2
            && let Some(entry) = self
                .browser
                .entries()
                .get(ix)
                .filter(|entry| entry.kind == EntryKind::Directory)
        {
            let destination = self
                .browser
                .path()
                .map(|parent| path::join(parent, &entry.name));
            self.load(destination, cx);
        } else if event.click_count == 2
            && self
                .browser
                .entries()
                .get(ix)
                .is_some_and(|entry| entry.kind == EntryKind::File)
            && let Some(target) = self.remote_file_target(ix)
        {
            open::later(
                target,
                self.presentation.host.workspace.clone(),
                self.presentation.host.installed.clone(),
                self.presentation.host.window,
                cx,
            );
        } else {
            cx.notify();
        }
    }

    fn select_for_menu(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.selected.contains(&ix) {
            self.selected.clear();
            self.selected.insert(ix);
            self.anchor = Some(ix);
            cx.notify();
        }
    }

    fn selection_for_row(&self, ix: usize) -> Vec<usize> {
        if self.selected.contains(&ix) {
            self.selected.iter().copied().collect()
        } else {
            vec![ix]
        }
    }

    pub(super) fn render_remote_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.browser.entries()[ix];
        let directory = entry.kind == EntryKind::Directory;
        let selected = self.selection_for_row(ix);
        let download = self.paths(selected.iter().copied());
        let visual = ExplorerRow {
            name: entry.name.clone(),
            directory: entry.kind == EntryKind::Directory,
            symlink: entry.is_symlink,
            selected: self.selected.contains(&ix),
            font_size: cx.design().typography.explorer_size.unwrap_or(12.0),
        };
        let source = nocterm_ui::DragSource::default();
        let destination = self
            .browser
            .path()
            .map(|parent| path::join(parent, &entry.name));
        let drop_path = destination;
        let enabled = self.filesystem().is_some() && !self.browser.loading();
        let target = self.remote_file_target(ix);
        let selected_targets = selected
            .into_iter()
            .filter_map(|ix| self.remote_file_target(ix))
            .collect::<Vec<_>>();
        let refresh = self.refresh_after_mutation(cx);
        let workspace = self.presentation.host.workspace.clone();
        let installed = self.presentation.host.installed.clone();
        visual
            .render(false, cx)
            .id(("remote-entry", ix))
            .test_support()
            .on_prepaint({
                let source = source.clone();
                move |bounds, window, _| source.capture(bounds, window)
            })
            .when(enabled, |row| row.cursor_pointer())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, _, cx| this.select_remote(ix, event, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| this.select_for_menu(ix, cx)),
            )
            .when_some(download, |row, download| {
                row.on_drag(FileDrag::Remote(download), move |_, _, _, cx| {
                    cx.stop_propagation();
                    let visual = visual.clone();
                    cx.new(|_| {
                        source.preview(move |_, cx| visual.render(true, cx).into_any_element())
                    })
                })
            })
            .when(directory && enabled, |row| {
                self.upload_target(row, drop_path.clone(), true, cx)
            })
            .context_menu(move |menu, _, cx| match &target {
                Some(target) => {
                    let menu = dialogs::menu(
                        menu,
                        target.clone(),
                        selected_targets.clone(),
                        refresh.clone(),
                        enabled,
                    );
                    open::menu_items(
                        menu,
                        target,
                        directory,
                        workspace.clone(),
                        installed.clone(),
                        cx,
                    )
                }
                None => menu,
            })
            .into_any_element()
    }

    fn upload_target<E: StatefulInteractiveElement>(
        &self,
        row: E,
        drop_path: Option<String>,
        stop_propagation: bool,
        cx: &mut Context<Self>,
    ) -> E {
        let upload_source = self.source.clone();
        let drop_path = drop_path.or_else(|| self.browser.path().map(str::to_owned));
        row.can_drop(|drag, _, _| {
            matches!(drag.downcast_ref::<FileDrag>(), Some(FileDrag::Local(_)))
                || drag.is::<ExternalPaths>()
        })
        .drag_over::<FileDrag>(|s, _, _, cx| {
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
            let source = upload_source.clone();
            move |_, files: &FileDrag, _, cx| {
                if stop_propagation {
                    cx.stop_propagation();
                }
                if let FileDrag::Local(paths) = files {
                    cx.emit(PaneIntent::Upload {
                        source: source.clone(),
                        sources: paths.clone(),
                        destination: drop_path.clone(),
                    });
                }
            }
        }))
        .on_drop(cx.listener(move |_, files: &ExternalPaths, _, cx| {
            if stop_propagation {
                cx.stop_propagation();
            }
            cx.emit(PaneIntent::Upload {
                source: upload_source.clone(),
                sources: files.paths().to_vec(),
                destination: drop_path.clone(),
            });
        }))
    }
    fn toolbar(&self, enabled: bool, cx: &mut Context<Self>) -> impl IntoElement {
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
                        !enabled || self.selected.is_empty() || !self.presentation.download_enabled,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(files) = this.paths(this.selected.iter().copied()) {
                            cx.emit(PaneIntent::Download {
                                files,
                                destination: None,
                            });
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
                    .disabled(!enabled || self.browser.path().is_none_or(|p| p == "/"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.load(this.browser.path().map(|p| path::parent(p).to_owned()), cx)
                    })),
            )
            .child(
                Button::new("files-refresh")
                    .small()
                    .ghost()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh remote directory")
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.load(this.requested.clone(), cx))),
            )
    }
    fn collision_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                        .selected(self.presentation.collisions == policy)
                        .tooltip(match policy {
                            CollisionPolicy::Skip => "Skip existing names",
                            CollisionPolicy::Rename => "Choose a new filename",
                            CollisionPolicy::Replace => {
                                "Replace existing files atomically if supported"
                            }
                        })
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(PaneIntent::Collision(policy));
                            cx.notify();
                        }))
                }),
            )
    }
    pub(super) fn render_remote(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.filesystem().is_some() && !self.browser.loading();
        let hint = RemotePresentation::hint(self.source.as_ref());
        let browser = v_flex()
            .id("remote-browser")
            .overflow_hidden()
            .size_full()
            .min_h_0()
            .gap_1()
            .px_2()
            .pb_2()
            .child(self.toolbar(enabled, cx))
            .child(self.path.clone())
            .when_some(hint, |p, hint| {
                p.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                )
            })
            .when(self.browser.loading(), |p| {
                p.child(div().text_sm().child("Loading directory…"))
            })
            .when_some(self.browser.error(), |p, e| {
                p.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(format!("{e}. Refresh to retry.")),
                )
            })
            .when(
                enabled && self.browser.entries().is_empty() && self.browser.error().is_none(),
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
                    self.browser.entries().len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| this.render_remote_row(ix, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .children(activity::strip(
                "remote-transfers",
                self.presentation.activity.iter(),
                cx,
            ))
            .when(self.browser.path().is_some() && enabled, |p| {
                p.child(
                    div()
                        .id("remote-statistics")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(statistics::summary(self.counter.shown())),
                )
            })
            .child(self.collision_controls(cx));
        self.upload_target(browser, None, false, cx)
            .into_any_element()
    }
}

impl Render for RemotePane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_remote(cx)
    }
}
