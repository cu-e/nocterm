//! The Explorer's remote half: browsing the active session's host.
use super::*;
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use row::ExplorerRow;

impl FilesPanel {
    pub(super) fn load(&mut self, directory: Option<String>, cx: &mut Context<Self>) {
        let Some(fs) = self.filesystem() else {
            return;
        };
        self.remote_path
            .update(cx, |input, cx| input.begin_navigation(cx));
        let generation = self.browser.begin();
        self.remote_selected.clear();
        self.remote_anchor = None;
        self.requested_directory = directory.clone();
        cx.notify();
        let window = self.window;
        let retry_directory = directory.clone();
        let home_requested = directory.is_none();
        self.remote_task = Some(cx.spawn(async move |this, cx| {
            let listing_fs = fs.clone();
            let result = cx
                .background_executor()
                .spawn(async move { listing(listing_fs, directory).await })
                .await;
            let error = this
                .update(cx, |this, cx| {
                    if this.browser.finish(generation, result) {
                        if this.browser.error.is_none() {
                            if home_requested {
                                this.browser.home = this.browser.path.clone();
                            }
                            if let Some(path) = this.browser.path.clone() {
                                let home =
                                    this.browser.home.clone().map(path_input::Directory::Remote);
                                this.remote_path.update(cx, |input, cx| {
                                    input.accept(path_input::Directory::Remote(path), home, cx)
                                });
                            }
                            this.count_remote(fs, cx);
                        }
                        if let Some(error) = this.browser.error.as_ref() {
                            let error = error.to_string();
                            this.remote_path
                                .update(cx, |input, cx| input.navigation_failed(error, cx));
                        }
                        cx.notify();
                        this.browser.error.as_ref().map(ToString::to_string)
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
                                if this.browser.generation == generation {
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
        let Some(directory) = self.browser.path.clone() else {
            return;
        };
        let indexing = indexing(cx);
        let skip = statistics::remote_skip(&directory, self.browser.home.as_deref(), &indexing);
        if let Some(skip) = skip {
            let kinds = self.browser.entries.iter().map(|entry| entry.kind);
            self.remote_counter.set(statistics::shallow(kinds, skip));
            return;
        }
        let (cancel, progress) = self.remote_counter.restart();
        let policy = statistics::Policy::remote(&indexing);
        cx.background_executor()
            .spawn(statistics::scan_remote(
                fs, directory, policy, cancel, progress,
            ))
            .detach();
    }

    pub(super) fn remote_file_target(&self, index: usize) -> Option<FileTarget> {
        let entry = self.browser.entries.get(index)?;
        let session = self.session.as_ref()?;
        Some(FileTarget::Remote {
            path: path::join(self.browser.path.as_deref()?, &entry.name),
            host: session.target.clone(),
            fs: session.fs.clone()?,
        })
    }

    pub(super) fn remote_paths(&self, indices: impl Iterator<Item = usize>) -> Option<RemotePaths> {
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

    pub(super) fn select_remote(
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
        } else if event.click_count == 2
            && self
                .browser
                .entries
                .get(ix)
                .is_some_and(|entry| entry.kind == EntryKind::File)
            && let Some(target) = self.remote_file_target(ix)
        {
            open::later(target, self.workspace.clone(), self.window, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn render_remote_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.browser.entries[ix];
        let directory = entry.kind == EntryKind::Directory;
        let selected: Vec<usize> = if self.remote_selected.contains(&ix) {
            self.remote_selected.iter().copied().collect()
        } else {
            vec![ix]
        };
        let download = self.remote_paths(selected.iter().copied());
        let visual = ExplorerRow {
            name: entry.name.clone(),
            directory: entry.kind == EntryKind::Directory,
            symlink: entry.is_symlink,
            selected: self.remote_selected.contains(&ix),
            font_size: cx.design().typography.explorer_size.unwrap_or(12.0),
        };
        let source = nocterm_ui::DragSource::default();
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
        let workspace = self.workspace.clone();
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
                row.on_drag(download, move |_, _, _, cx| {
                    cx.stop_propagation();
                    let visual = visual.clone();
                    cx.new(|_| {
                        source.preview(move |_, cx| visual.render(true, cx).into_any_element())
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
                    move |this, files: &LocalPaths, window, cx| {
                        cx.stop_propagation();
                        this.enqueue(files.0.clone(), drop_path.clone(), window, cx);
                    }
                }))
                .on_drop(cx.listener(
                    move |this, files: &ExternalPaths, window, cx| {
                        cx.stop_propagation();
                        this.enqueue(files.paths().to_vec(), drop_path.clone(), window, cx);
                    },
                ))
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
                    open::menu_items(menu, target, directory, workspace.clone(), cx)
                }
                None => menu,
            })
            .into_any_element()
    }

    pub(super) fn render_remote(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.filesystem().is_some() && !self.browser.loading;
        let hint = match &self.session {
            None => Some("Open a connection to browse remote files."),
            Some(s) if !s.connected => Some("Disconnected. Reconnect to browse files."),
            Some(s) if s.fs.is_none() => Some("This host does not provide SFTP."),
            _ => None,
        };
        v_flex()
            .id("remote-browser")
            .overflow_hidden()
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(files) =
                                    this.remote_paths(this.remote_selected.iter().copied())
                                {
                                    this.enqueue_download(files, None, window, cx);
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
            .child(self.remote_path.clone())
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
            .children(self.session.as_ref().and_then(|session| {
                activity::strip(
                    "remote-transfers",
                    self.activity.uploads(&session.target),
                    cx,
                )
            }))
            .when(self.browser.path.is_some() && enabled, |p| {
                p.child(
                    div()
                        .id("remote-statistics")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(statistics::summary(&self.remote_counter.shown)),
                )
            })
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
            .on_drop(cx.listener(|this, files: &LocalPaths, window, cx| {
                this.enqueue(files.0.clone(), None, window, cx)
            }))
            .on_drop(cx.listener(|this, files: &ExternalPaths, window, cx| {
                this.enqueue(files.paths().to_vec(), None, window, cx)
            }))
            .into_any_element()
    }
}
