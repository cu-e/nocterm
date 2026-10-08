//! The Explorer's local half: browsing this computer.
use super::*;
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use row::ExplorerRow;

impl FilesPanel {
    pub(super) fn load_local(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.load_local_with_navigation(directory, path_input::NavigationMode::CloseEditor, cx);
    }

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn load_local_with_navigation(
        &mut self,
        directory: PathBuf,
        mode: path_input::NavigationMode,
        cx: &mut Context<Self>,
    ) {
        self.local_path
            .update(cx, |input, cx| input.begin_navigation_with_mode(mode, cx));
        let (cancel, progress) = self.local.counter.restart();
        self.local.generation = self.local.generation.wrapping_add(1);
        let generation = self.local.generation;
        self.local.loading = true;
        self.local.error = None;
        self.local.requested = Some(directory.clone());
        self.statistics_reported = false;
        let indexing = indexing(cx);
        cx.notify();
        let window = self.window;
        self.local_task = Some(cx.spawn(async move |this, cx| {
            let path = directory.clone();
            let listing_cancel = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move { local::read_directory(&path, &listing_cancel) })
                .await;
            let outcome = this
                .update(cx, |this, cx| {
                    if this.local.generation != generation {
                        return None;
                    }
                    this.local.loading = false;
                    let mut walk = false;
                    match result {
                        Ok(entries) => {
                            let home = local::home();
                            match statistics::local_skip(&directory, home.as_deref(), &indexing) {
                                Some(skip) => this.local.counter.set(statistics::shallow(
                                    entries.iter().map(|entry| entry.kind),
                                    skip,
                                )),
                                None => walk = true,
                            }
                            this.local_path.update(cx, |input, cx| {
                                input.accept(
                                    path_input::Directory::Local(directory.clone()),
                                    home.map(path_input::Directory::Local),
                                    cx,
                                )
                            });
                            this.local.path = Some(directory.clone());
                            this.local.entries = entries;
                            this.local.selected.clear();
                            this.local.anchor = None;
                        }
                        Err(error) => {
                            this.local_path
                                .update(cx, |input, cx| input.navigation_failed(error.clone(), cx));
                            this.local.error = Some(error);
                        }
                    }
                    cx.notify();
                    Some(this.local.error.clone().ok_or(walk))
                })
                .ok()
                .flatten();
            match outcome {
                Some(Err(true)) => {
                    let policy = statistics::Policy::local(&indexing);
                    cx.background_executor()
                        .spawn(async move {
                            local::scan(directory, &policy, cancel, progress);
                        })
                        .await;
                }
                Some(Err(false)) => {}
                Some(Ok(error)) => {
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
                                    if this.local.generation == generation {
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

    pub(super) fn select_local(
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
        } else if event.click_count == 2
            && let Some(entry) = self
                .local
                .entries
                .get(ix)
                .filter(|e| e.kind == EntryKind::File)
        {
            let target = FileTarget::Local(entry.path.clone());
            open::later(target, self.workspace.clone(), self.window, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn local_paths(&self, index: usize) -> Vec<PathBuf> {
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

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn render_local_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.local.entries[ix];
        let paths = self.local_paths(ix);
        let visual = ExplorerRow {
            name: entry.name.clone(),
            directory: entry.kind == EntryKind::Directory,
            symlink: entry.symlink,
            selected: self.local.selected.contains(&ix),
            font_size: cx.design().typography.explorer_size.unwrap_or(12.0),
        };
        let source = nocterm_ui::DragSource::default();
        let destination = entry.path.clone();
        let directory = entry.kind == EntryKind::Directory && !entry.symlink;
        let target = FileTarget::Local(entry.path.clone());
        let selected_targets = paths
            .iter()
            .cloned()
            .map(FileTarget::Local)
            .collect::<Vec<_>>();
        let refresh = self.refresh_after_mutation(true, cx);
        let workspace = self.workspace.clone();
        let folder = entry.kind == EntryKind::Directory;
        let enabled = !self.local.loading;
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
                cx.listener(move |this, _, _, cx| {
                    if !this.local.selected.contains(&ix) {
                        this.local.selected.clear();
                        this.local.selected.insert(ix);
                        this.local.anchor = Some(ix);
                        cx.notify();
                    }
                }),
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
                .on_drop(cx.listener(
                    move |this, files: &FileDrag, window, cx| {
                        cx.stop_propagation();
                        if let FileDrag::Remote(files) = files {
                            this.enqueue_download(
                                files.clone(),
                                Some(destination.clone()),
                                window,
                                cx,
                            );
                        }
                    },
                ))
            })
            .context_menu(move |menu, _, cx| {
                let menu = dialogs::menu(
                    menu,
                    target.clone(),
                    selected_targets.clone(),
                    refresh.clone(),
                    enabled,
                );
                open::menu_items(menu, &target, folder, workspace.clone(), cx)
            })
            .into_any_element()
    }

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn render_local(&self, cx: &mut Context<Self>) -> AnyElement {
        let cwd = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).local_terminal_cwd(cx));
        let information = statistics::summary(&self.local.counter.shown);
        v_flex()
            .id("local-browser")
            .overflow_hidden()
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
            .child(self.local_path.clone())
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
            .children(activity::strip(
                "local-transfers",
                self.activity.downloads(),
                cx,
            ))
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
                                            if let Err(error) = result {
                                                nocterm_ui::notice::error(
                                                    window,
                                                    cx,
                                                    "files-shell-directory",
                                                    "Could not change shell directory",
                                                    error,
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
            .can_drop(|drag, _, _| {
                matches!(drag.downcast_ref::<FileDrag>(), Some(FileDrag::Remote(_)))
            })
            .drag_over::<FileDrag>(|style, _, _, cx| {
                style
                    .bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().primary)
            })
            .on_drop(cx.listener(|this, files: &FileDrag, window, cx| {
                cx.stop_propagation();
                if let FileDrag::Remote(files) = files {
                    this.enqueue_download(files.clone(), None, window, cx);
                }
            }))
            .into_any_element()
    }
}
