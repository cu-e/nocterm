//! Editable Explorer location, with asynchronous Tab completion below the field.
mod completion;
mod popup;
#[cfg(test)]
mod tests;

pub(super) use completion::Directory;
use completion::{Candidate, Cycle, Query, query, resolve};
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    KeyBinding, ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        input::{self, Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_session::{EntryKind, RemoteFs};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const KEY_CONTEXT: &str = "ExplorerPathInput";
struct Bindings;
impl Global for Bindings {}

pub(super) struct Navigate(pub Directory);

pub(super) struct PathInput {
    input: Entity<InputState>,
    window: AnyWindowHandle,
    focus: FocusHandle,
    directory: Option<Directory>,
    home: Option<Directory>,
    fs: Option<Arc<dyn RemoteFs>>,
    editing: bool,
    navigating: bool,
    generation: u64,
    cycle: Option<Cycle>,
    cache: Option<(Directory, Vec<Candidate>)>,
    pending: Option<(Query, i64)>,
    observed_value: String,
    error: Option<String>,
    encoding_error: Option<String>,
    cancel: Arc<AtomicBool>,
    task: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}
impl EventEmitter<Navigate> for PathInput {}
impl PathInput {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<Bindings>() {
            cx.bind_keys([
                KeyBinding::new("tab", input::IndentInline, Some(KEY_CONTEXT)),
                KeyBinding::new("shift-tab", input::OutdentInline, Some(KEY_CONTEXT)),
                KeyBinding::new(
                    "enter",
                    input::Enter {
                        shift: false,
                        secondary: false,
                    },
                    Some(KEY_CONTEXT),
                ),
                KeyBinding::new("escape", input::Escape, Some(KEY_CONTEXT)),
            ]);
            cx.set_global(Bindings);
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Directory path"));
        let subscription = cx.subscribe(&input, |this, input, event, cx| match event {
            InputEvent::Change => {
                let value = input.read(cx).value();
                if value.as_ref() == this.observed_value {
                    return;
                }
                this.observed_value = value.to_string();
                this.invalidate(false);
                this.error = None;
                cx.notify();
            }
            InputEvent::Blur if !this.navigating => {
                this.invalidate(false);
                this.editing = false;
                this.error = None;
                cx.notify();
            }
            _ => {}
        });
        Self {
            input,
            window: window.window_handle(),
            focus: cx.focus_handle(),
            directory: None,
            home: None,
            fs: None,
            editing: false,
            navigating: false,
            generation: 0,
            cycle: None,
            cache: None,
            pending: None,
            observed_value: String::new(),
            error: None,
            encoding_error: None,
            cancel: Arc::new(AtomicBool::new(false)),
            task: None,
            scroll: UniformListScrollHandle::default(),
            _subscription: subscription,
        }
    }

    pub(super) fn reset_remote(&mut self, fs: Option<Arc<dyn RemoteFs>>, cx: &mut Context<Self>) {
        self.invalidate(true);
        self.directory = None;
        self.home = None;
        self.fs = fs;
        self.editing = false;
        self.navigating = false;
        self.error = None;
        self.encoding_error = None;
        cx.notify();
    }

    /// Called for every listing, including refresh and other navigation controls.
    pub(super) fn begin_navigation(&mut self, cx: &mut Context<Self>) {
        self.invalidate(true);
        self.navigating = true;
        self.error = None;
        self.encoding_error = None;
        cx.notify();
    }

    pub(super) fn accept(
        &mut self,
        directory: Directory,
        home: Option<Directory>,
        cx: &mut Context<Self>,
    ) {
        self.invalidate(true);
        if self.editing {
            let field = self.input.read(cx).focus_handle(cx);
            let display = self.focus.clone();
            let handle = self.window;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    if field.is_focused(window) {
                        window.focus(&display, cx);
                    }
                });
            });
        }
        self.directory = Some(directory);
        self.home = home;
        self.editing = false;
        self.navigating = false;
        self.error = None;
        self.encoding_error = None;
        cx.notify();
    }

    pub(super) fn navigation_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.navigating = false;
        self.error = Some(error);
        cx.notify();
    }

    fn invalidate(&mut self, cache: bool) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel.store(true, Ordering::Release);
        self.task = None;
        self.pending = None;
        self.cycle = None;
        if cache {
            self.cache = None;
        }
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = &self.directory else {
            return;
        };
        if self.navigating {
            return;
        }
        let value = match directory {
            Directory::Local(path) => match path.to_str() {
                Some(path) => path.to_owned(),
                None => {
                    self.encoding_error =
                        Some("This directory path cannot be represented as text.".into());
                    cx.notify();
                    return;
                }
            },
            Directory::Remote(path) => path.clone(),
        };
        self.invalidate(false);
        self.error = None;
        self.editing = true;
        self.encoding_error = None;
        self.observed_value = value.clone();
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.navigating {
            return;
        }
        let Some(base) = &self.directory else {
            return;
        };
        match resolve(
            self.input.read(cx).value().as_ref(),
            base,
            self.home.as_ref(),
        ) {
            Ok(directory) => {
                self.begin_navigation(cx);
                cx.emit(Navigate(directory));
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
        let _ = window;
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.invalidate(false);
        self.editing = false;
        self.error = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn complete(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.navigating {
            return;
        }
        let value = self.input.read(cx).value().to_string();
        // Change notifications may be deferred until after this action. Synchronize
        // the draft so an earlier Change cannot cancel the newly started request.
        if value != self.observed_value {
            self.invalidate(false);
            self.observed_value = value;
        }
        if let Some(cycle) = &self.cycle {
            if cycle.candidates.len() == 1 && cycle.candidates[0].directory {
                self.cycle = None;
            } else {
                self.advance(reverse, window, cx);
                return;
            }
        }
        if let Some((_, position)) = &mut self.pending {
            *position = position.saturating_add(if reverse { -1 } else { 1 });
            return;
        }
        let Some(base) = &self.directory else {
            return;
        };
        let query = match query(
            self.input.read(cx).value().as_ref(),
            base,
            self.home.as_ref(),
        ) {
            Ok(query) => query,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.error = None;
        if let Some((directory, entries)) = &self.cache
            && directory == &query.directory
        {
            self.cycle = Some(Cycle::new(query, entries));
            self.advance(reverse, window, cx);
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let directory = query.directory.clone();
        let filesystem = self.fs.clone();
        self.pending = Some((query, if reverse { -1 } else { 0 }));
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { read_candidates(directory, filesystem, cancel).await })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                let Some((query, position)) = this.pending.take() else {
                    return;
                };
                match result {
                    Ok(entries) => {
                        this.cache = Some((query.directory.clone(), entries.clone()));
                        let mut cycle = Cycle::new(query, &entries);
                        let value = if cycle.candidates.is_empty() {
                            None
                        } else {
                            cycle
                                .choose(position.rem_euclid(cycle.candidates.len() as i64) as usize)
                        };
                        this.cycle = Some(cycle);
                        if let Some(value) = value {
                            this.apply(value, window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn advance(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(value) = self.cycle.as_mut().and_then(|cycle| cycle.step(reverse)) {
            self.apply(value, window, cx);
        }
        cx.notify();
    }
    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(value) = self.cycle.as_mut().and_then(|cycle| cycle.choose(index)) {
            self.apply(value, window, cx);
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }
    fn apply(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        // Preserve native undo and ignore the Change caused by our own replacement.
        self.observed_value = value.clone();
        self.input
            .update(cx, |input, cx| input.replace_all(value, window, cx));
        if let Some(index) = self.cycle.as_ref().and_then(|cycle| cycle.selected) {
            self.scroll.scroll_to_item(index, ScrollStrategy::Top);
        }
    }
}

impl Focusable for PathInput {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.editing {
            self.input.read(cx).focus_handle(cx)
        } else {
            self.focus.clone()
        }
    }
}
impl Render for PathInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown: SharedString = self
            .directory
            .as_ref()
            .map(|directory| match directory {
                Directory::Local(path) => path.display().to_string(),
                Directory::Remote(path) => path.clone(),
            })
            .unwrap_or_default()
            .into();
        v_flex()
            .id("explorer-path")
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if !this.editing && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        this.edit(window, cx);
                    }
                }),
            )
            .w_full()
            .min_w_0()
            .min_h_0()
            .gap_1()
            .when(self.directory.is_some(), |view| {
                if self.editing {
                    view.child(
                        div()
                            .key_context(KEY_CONTEXT)
                            .capture_action(cx.listener(
                                |this, _: &input::IndentInline, window, cx| {
                                    this.complete(false, window, cx)
                                },
                            ))
                            .capture_action(cx.listener(
                                |this, _: &input::OutdentInline, window, cx| {
                                    this.complete(true, window, cx)
                                },
                            ))
                            .capture_action(cx.listener(|this, _: &input::Enter, window, cx| {
                                this.submit(window, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &input::Escape, window, cx| {
                                this.escape(window, cx)
                            }))
                            .child(Input::new(&self.input).small().disabled(self.navigating)),
                    )
                } else {
                    view.child(
                        div()
                            .id("explorer-path-display")
                            .test_support()
                            .w_full()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .when(self.focus.is_focused(window), |path| {
                                path.bg(cx.theme().accent)
                                    .border_1()
                                    .border_color(cx.theme().primary)
                            })
                            .text_xs()
                            .text_color(cx.theme().foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .when(!self.navigating, |path| {
                                path.cursor_text().hover(|path| path.bg(cx.theme().accent))
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx)))
                            .child(shown),
                    )
                }
            })
            .children(self.render_popup(window, cx))
            .when_some(
                self.encoding_error
                    .clone()
                    .or_else(|| self.error.clone().filter(|_| self.editing)),
                |view, error| {
                    view.child(
                        div()
                            .id("explorer-path-error")
                            .test_support()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(error),
                    )
                },
            )
    }
}

async fn read_candidates(
    directory: Directory,
    fs: Option<Arc<dyn RemoteFs>>,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<Candidate>, String> {
    match directory {
        Directory::Local(path) => super::local::read_directory(&path, &cancel).map(|entries| {
            entries
                .into_iter()
                .filter(|entry| {
                    entry
                        .path
                        .file_name()
                        .is_some_and(|name| name.to_str().is_some())
                })
                .filter(|entry| valid_name(&entry.name, true))
                .map(|entry| {
                    let directory = entry.kind == EntryKind::Directory
                        || (entry.symlink
                            && std::fs::metadata(&entry.path)
                                .is_ok_and(|metadata| metadata.is_dir()));
                    Candidate {
                        name: entry.name,
                        directory,
                    }
                })
                .collect()
        }),
        Directory::Remote(path) => {
            let fs = fs.ok_or_else(|| "Reconnect to load remote suggestions.".to_owned())?;
            super::remote::listing(fs, Some(path))
                .await
                .map(|(_, entries)| {
                    entries
                        .into_iter()
                        .filter(|entry| valid_name(&entry.name, false))
                        .map(|entry| Candidate {
                            name: entry.name,
                            directory: entry.kind == EntryKind::Directory,
                        })
                        .collect()
                })
                .map_err(|error| error.to_string())
        }
    }
}

fn valid_name(name: &str, local: bool) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !(local && cfg!(windows) && name.contains('\\'))
        && !name.chars().any(char::is_control)
}
