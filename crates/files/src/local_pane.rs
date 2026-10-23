//! The Explorer's local half: browsing this computer.
use super::*;
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use row::ExplorerRow;

use nocterm_session::local_fs::{LocalFs, LocalMutations};
use nocterm_transfers::Progress;
enum NavigationOutcome {
    Scan,
    Done,
    Failed(String),
}
struct LocalPorts {
    fs: Arc<dyn LocalFs>,
    mutations: Arc<dyn LocalMutations>,
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
    home: Option<PathBuf>,
}

pub(super) struct LocalPane {
    browser: LocalBrowser,
    path: Entity<path_input::PathInput>,
    _subscription: Subscription,
    task: Option<Task<()>>,
    statistics_reported: bool,
    stale: bool,
    ports: LocalPorts,
    host: PaneHost,
    activity: Vec<Progress>,
}
impl EventEmitter<PaneIntent> for LocalPane {}
impl LocalPane {
    pub(super) fn new(
        installed: &Installed,
        host: PaneHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let path =
            cx.new(|cx| path_input::PathInput::new(installed.directories().local(), window, cx));
        let subscription = cx.subscribe(&path, |this, _, event: &path_input::Navigate, cx| {
            if let path_input::Directory::Local(path) = &event.directory {
                this.load_local_with_navigation(path.clone(), event.mode, cx);
            }
        });
        let mut pane = Self {
            browser: LocalBrowser::default(),
            path,
            _subscription: subscription,
            task: None,
            statistics_reported: false,
            stale: false,
            ports: LocalPorts {
                fs: installed.local().clone(),
                mutations: installed.mutations().clone(),
            },
            host,
            activity: Vec::new(),
        };
        let initial = pane
            .host
            .installed
            .settings()
            .setting::<nocterm_session::LocalShellSettings>(cx)
            .cwd
            .clone()
            .map(PathBuf::from);
        let local = pane.ports.fs.clone();
        pane.task = Some(cx.spawn(async move |this, cx| {
            let home = cx
                .background_executor()
                .spawn(async move { local.home() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.browser.home = home.ok().flatten();
                if this.browser.requested.is_none()
                    && let Some(path) = initial.or_else(|| this.browser.home.clone())
                {
                    this.load_local(path, cx);
                }
            });
        }));
        pane
    }
    pub(super) fn listing(&self) -> ListingState<PathBuf> {
        ListingState {
            path: self.browser.path.clone(),
            requested: self.browser.requested.clone(),
            names: self
                .browser
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
            selected: self.browser.selected.iter().copied().collect(),
            loading: self.browser.loading,
            error: self.browser.error.clone(),
            statistics: statistics::summary(self.browser.counter.shown()),
        }
    }
    pub(super) fn directory(&self) -> Option<&std::path::Path> {
        self.browser.path.as_deref()
    }
    pub(super) fn ready(&self) -> bool {
        self.browser.path.is_some() && !self.browser.loading
    }
    pub(super) fn transfers(
        &mut self,
        jobs: Vec<Progress>,
        finished: &[Progress],
        cx: &mut Context<Self>,
    ) {
        if self.browser.path.as_ref().is_some_and(|shown| {
            finished
                .iter()
                .any(|job| activity::changes_local(job, shown))
        }) {
            self.stale = true;
        }
        if self.activity != jobs {
            self.activity = jobs;
            cx.notify();
        }
        if self.stale
            && !self.browser.loading
            && let Some(shown) = self.browser.path.clone()
        {
            self.stale = false;
            self.load_local(shown, cx);
        }
    }
    fn refresh_after_mutation(&self, cx: &Context<Self>) -> dialogs::Refresh {
        let pane = cx.entity().downgrade();
        let generation = self.browser.generation;
        Rc::new(move |cx| {
            let _ = pane.update(cx, |this, cx| {
                if this.browser.generation == generation
                    && let Some(path) = this.browser.path.clone()
                {
                    this.load_local(path, cx);
                }
            });
        })
    }
    pub(super) fn poll_statistics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.browser.counter.poll() {
            return;
        }
        cx.notify();
        let statistics = self.browser.counter.shown();
        if !statistics.complete || statistics.inaccessible == 0 || self.statistics_reported {
            return;
        }
        self.statistics_reported = true;
        let Some(path) = self.browser.path.clone() else {
            return;
        };
        let message = statistics.errors.first().cloned().unwrap_or_else(|| {
            format!(
                "{} paths could not be scanned; folder size is partial.",
                statistics.inaccessible
            )
        });
        let generation = self.browser.generation;
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
                    if this.browser.generation == generation
                        && this.browser.path.as_ref() == Some(&path)
                    {
                        this.load_local(path.clone(), cx);
                    }
                });
            },
        );
    }
}
impl LocalPane {
    pub(super) fn load_local(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.load_local_with_navigation(directory, path_input::NavigationMode::CloseEditor, cx);
    }

    pub(super) fn load_local_with_navigation(
        &mut self,
        directory: PathBuf,
        mode: path_input::NavigationMode,
        cx: &mut Context<Self>,
    ) {
        self.path
            .update(cx, |input, cx| input.begin_navigation_with_mode(mode, cx));
        let (cancel, progress) = self.browser.counter.restart();
        self.browser.generation = self.browser.generation.wrapping_add(1);
        let generation = self.browser.generation;
        self.browser.loading = true;
        self.browser.error = None;
        self.browser.requested = Some(directory.clone());
        self.statistics_reported = false;
        cx.emit(PaneIntent::Changed);
        let indexing = indexing(&self.host.installed, cx);
        cx.notify();
        let window = self.host.window;
        let local = self.ports.fs.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let path = directory.clone();
            let listing_cancel = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    local
                        .browse(&path, Default::default(), &listing_cancel)
                        .map(local::entries)
                })
                .await;
            let outcome = this
                .update(cx, |this, cx| {
                    this.finish_navigation(generation, &directory, result, &indexing, cx)
                })
                .ok()
                .flatten();
            match outcome {
                Some(NavigationOutcome::Scan) => {
                    let local = this.read_with(cx, |this, _| this.ports.fs.clone()).ok();
                    let Some(local) = local else {
                        return;
                    };
                    cx.background_executor()
                        .spawn(async move {
                            scan_local(local.as_ref(), &directory, &indexing, &cancel, &progress);
                        })
                        .await;
                }
                Some(NavigationOutcome::Done) => {}
                Some(NavigationOutcome::Failed(error)) => {
                    let panel = this.clone();
                    let _ = window.update(cx, |_, window, cx| {
                        nocterm_ui::notice::error_action(
                            window,
                            cx,
                            "files-local-list",
                            "Could not load local directory",
                            error,
                            "Retry",
                            move |_, cx| {
                                let _ = panel.update(cx, |this, cx| {
                                    if this.browser.generation == generation {
                                        this.load_local(directory.clone(), cx);
                                    }
                                });
                            },
                        );
                    });
                }
                None => {}
            }
        }));
    }

    fn finish_navigation(
        &mut self,
        generation: u64,
        directory: &std::path::Path,
        result: nocterm_session::local_fs::LocalResult<Vec<local::LocalEntry>>,
        indexing: &IndexingSettings,
        cx: &mut Context<Self>,
    ) -> Option<NavigationOutcome> {
        if self.browser.generation != generation {
            return None;
        }
        self.browser.loading = false;
        let mut walk = false;
        match result {
            Ok(entries) => {
                let home = self.browser.home.clone();
                match statistics::local_skip(directory, home.as_deref(), indexing) {
                    Some(skip) => self.browser.counter.set(statistics::shallow(
                        entries.iter().map(|entry| entry.kind),
                        skip,
                    )),
                    None => walk = true,
                }
                self.path.update(cx, |input, cx| {
                    input.accept(
                        path_input::Directory::Local(directory.to_owned()),
                        home.map(path_input::Directory::Local),
                        cx,
                    )
                });
                self.browser.path = Some(directory.to_owned());
                self.browser.entries = entries;
                self.browser.selected.clear();
                self.browser.anchor = None;
            }
            Err(error) => {
                self.path.update(cx, |input, cx| {
                    input.navigation_failed(error.to_string(), cx)
                });
                self.browser.error = Some(error.to_string());
            }
        }
        cx.notify();
        cx.emit(PaneIntent::Changed);
        Some(match self.browser.error.clone() {
            Some(error) => NavigationOutcome::Failed(error),
            None if walk => NavigationOutcome::Scan,
            None => NavigationOutcome::Done,
        })
    }
    pub(super) fn select_local(
        &mut self,
        ix: usize,
        event: &gpui_kit::MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.shift {
            let anchor = self.browser.anchor.unwrap_or(ix);
            self.browser
                .selected
                .extend(anchor.min(ix)..=anchor.max(ix));
        } else if event.modifiers.control || event.modifiers.platform {
            if !self.browser.selected.insert(ix) {
                self.browser.selected.remove(&ix);
            }
            self.browser.anchor = Some(ix);
        } else {
            self.browser.selected.clear();
            self.browser.selected.insert(ix);
            self.browser.anchor = Some(ix);
        }
        if event.click_count == 2
            && let Some(entry) = self
                .browser
                .entries
                .get(ix)
                .filter(|e| e.kind == EntryKind::Directory && !e.symlink)
        {
            self.load_local(entry.path.clone(), cx);
        } else if event.click_count == 2
            && let Some(entry) = self
                .browser
                .entries
                .get(ix)
                .filter(|e| e.kind == EntryKind::File)
        {
            let target = FileTarget::local(entry.path.clone(), self.ports.mutations.clone());
            open::later(
                target,
                self.host.workspace.clone(),
                self.host.installed.clone(),
                self.host.window,
                cx,
            );
        } else {
            cx.notify();
        }
    }

    pub(super) fn paths(&self, index: usize) -> Vec<PathBuf> {
        if self.browser.selected.contains(&index) {
            self.browser
                .selected
                .iter()
                .filter_map(|ix| self.browser.entries.get(*ix))
                .map(|e| e.path.clone())
                .collect()
        } else {
            self.browser
                .entries
                .get(index)
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default()
        }
    }

    fn select_for_menu(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.browser.selected.contains(&ix) {
            self.browser.selected.clear();
            self.browser.selected.insert(ix);
            self.browser.anchor = Some(ix);
            cx.notify();
        }
    }

    pub(super) fn render_local_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.browser.entries[ix];
        let paths = self.paths(ix);
        let visual = ExplorerRow::local(entry, self.browser.selected.contains(&ix), cx);
        let source = nocterm_ui::DragSource::default();
        let destination = entry.path.clone();
        let directory = entry.kind == EntryKind::Directory && !entry.symlink;
        let target = FileTarget::local(entry.path.clone(), self.ports.mutations.clone());
        let selected_targets = paths
            .iter()
            .cloned()
            .map(|path| FileTarget::local(path, self.ports.mutations.clone()))
            .collect::<Vec<_>>();
        let refresh = self.refresh_after_mutation(cx);
        let workspace = self.host.workspace.clone();
        let installed = self.host.installed.clone();
        let folder = entry.kind == EntryKind::Directory;
        let enabled = !self.browser.loading;
        visual
            .render(false, cx)
            .id(("local-entry", ix))
            .test_support()
            .on_prepaint({
                let source = source.clone();
                move |bounds, window, _| source.capture(bounds, window)
            })
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, _, cx| this.select_local(ix, event, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| this.select_for_menu(ix, cx)),
            )
            .on_drag(FileDrag::Local(paths), move |_, _, _, cx| {
                cx.stop_propagation();
                let visual = visual.clone();
                cx.new(|_| source.preview(move |_, cx| visual.render(true, cx).into_any_element()))
            })
            .when(directory, |row| {
                row.can_drop(|drag, _, _| {
                    matches!(drag.downcast_ref::<FileDrag>(), Some(FileDrag::Remote(_)))
                })
                .drag_over::<FileDrag>(|style, _, _, cx| {
                    style
                        .bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().primary)
                })
                .on_drop(cx.listener(move |_, files: &FileDrag, _, cx| {
                    cx.stop_propagation();
                    if let FileDrag::Remote(files) = files {
                        cx.emit(PaneIntent::Download {
                            files: files.clone(),
                            destination: Some(destination.clone()),
                        });
                    }
                }))
            })
            .context_menu(move |menu, _, cx| {
                let menu = dialogs::menu(
                    menu,
                    target.clone(),
                    selected_targets.clone(),
                    refresh.clone(),
                    enabled,
                );
                open::menu_items(
                    menu,
                    &target,
                    folder,
                    workspace.clone(),
                    installed.clone(),
                    cx,
                )
            })
            .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .child(div().flex_1().text_sm().child("Local"))
            .child(path_input::DirectoryNavigation::new(
                "local",
                !self.browser.loading,
                self.browser
                    .path
                    .as_deref()
                    .and_then(|path| path.parent())
                    .is_some(),
                {
                    let pane = cx.entity().downgrade();
                    move |action, _, cx| {
                        let _ = pane.update(cx, |pane, cx| {
                            let directory = match action {
                                path_input::NavigationAction::Home => pane.browser.home.clone(),
                                path_input::NavigationAction::Up => pane
                                    .browser
                                    .path
                                    .as_deref()
                                    .and_then(|path| path.parent())
                                    .map(ToOwned::to_owned),
                                path_input::NavigationAction::Refresh => {
                                    pane.browser.requested.clone()
                                }
                            };
                            if let Some(directory) = directory {
                                pane.load_local(directory, cx);
                            }
                        });
                    }
                },
            ))
    }
    fn footer(&self, cwd: Option<PathBuf>, cx: &mut Context<Self>) -> impl IntoElement {
        let information = statistics::summary(self.browser.counter.shown());
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
                            .disabled(self.browser.path.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let (Some(path), Some(workspace)) =
                                    (this.browser.path.clone(), this.host.workspace.upgrade())
                                {
                                    let result = workspace.update(cx, |workspace, cx| {
                                        workspace.change_local_directory(path, window, cx)
                                    });
                                    if let Err(error) = result {
                                        nocterm_ui::notice::error(
                                            window,
                                            cx,
                                            "files-shell-directory",
                                            "Could not change shell directory",
                                            error.to_string(),
                                        );
                                    }
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
                                    this.host.workspace.upgrade().and_then(|workspace| {
                                        workspace.read(cx).local_terminal_cwd(cx)
                                    })
                                {
                                    this.load_local(path, cx);
                                }
                            })),
                    ),
            )
    }
    pub(super) fn render_local(&self, cx: &mut Context<Self>) -> AnyElement {
        let cwd = self
            .host
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).local_terminal_cwd(cx));
        v_flex()
            .id("local-browser")
            .overflow_hidden()
            .size_full()
            .min_h_0()
            .gap_1()
            .px_2()
            .pt_2()
            .child(self.toolbar(cx))
            .child(self.path.clone())
            .when(self.browser.loading, |p| {
                p.child(div().text_sm().child("Loading directory…"))
            })
            .when_some(self.browser.error.as_ref(), |p, e| {
                p.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(e.clone()),
                )
            })
            .when(
                !self.browser.loading
                    && self.browser.entries.is_empty()
                    && self.browser.error.is_none(),
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
                    self.browser.entries.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| this.render_local_row(ix, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .children(activity::strip("local-transfers", self.activity.iter(), cx))
            .child(self.footer(cwd, cx))
            .can_drop(|drag, _, _| {
                matches!(drag.downcast_ref::<FileDrag>(), Some(FileDrag::Remote(_)))
            })
            .drag_over::<FileDrag>(|style, _, _, cx| {
                style
                    .bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().primary)
            })
            .on_drop(cx.listener(|_, files: &FileDrag, _, cx| {
                cx.stop_propagation();
                if let FileDrag::Remote(files) = files {
                    cx.emit(PaneIntent::Download {
                        files: files.clone(),
                        destination: None,
                    });
                }
            }))
            .into_any_element()
    }
}

impl Render for LocalPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_local(cx)
    }
}

fn scan_local(
    local: &dyn LocalFs,
    directory: &std::path::Path,
    indexing: &IndexingSettings,
    cancel: &nocterm_session::local_fs::CancelFlag,
    progress: &Arc<parking_lot::Mutex<statistics::Statistics>>,
) {
    let policy = nocterm_session::local_fs::ScanPolicy::new(
        indexing
            .excluded
            .iter()
            .map(std::ffi::OsString::from)
            .collect(),
        indexing.max_local_entries as usize,
        statistics::MAX_SCAN_DEPTH,
    );
    let observer = |value: nocterm_session::local_fs::Statistics| {
        *progress.lock() = value.into();
    };
    let result = policy.and_then(|policy| local.scan(directory, &policy, cancel, &observer));
    match result {
        Ok(value) => observer(value),
        Err(error) => {
            let mut shown = progress.lock();
            statistics::record_error(&mut shown, error.to_string());
            shown.complete = true;
        }
    }
}
