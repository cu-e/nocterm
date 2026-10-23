//! Editable Explorer location, with asynchronous Tab completion below the field.
#[cfg(test)]
mod navigation_tests;
mod popup;
mod state;
#[cfg(test)]
mod tests;

use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        input::{self, Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_explorer::directory::Directory;
use nocterm_explorer::directory::{
    Candidate, Cycle, DirectoryError, DirectoryRead, Query, query, resolve,
};
use nocterm_session::local_fs::CancelFlag;
use state::{CompletionState, LocationState, NavigationState};
use std::sync::Arc;

const KEY_CONTEXT: &str = "DirectoryPathInput";

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum NavigationMode {
    #[default]
    CloseEditor,
    KeepEditing,
}
pub struct Navigate {
    pub directory: Directory,
    pub mode: NavigationMode,
}

// The display and native editor share actual pixel metrics, including border space.
fn path_metrics<T: Styled>(element: T) -> T {
    element
        .text_size(gpui_kit::px(12.))
        .line_height(gpui_kit::px(18.))
        .h(gpui_kit::px(28.))
        .px(gpui_kit::px(8.))
        .py(gpui_kit::px(4.))
}

pub struct PathInput {
    input: Entity<InputState>,
    window: AnyWindowHandle,
    focus: FocusHandle,
    location: LocationState,
    navigation: NavigationState,
    completion: CompletionState,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}
/// A native or completion edit invalidates validation in the parent form.
pub enum PathInputEvent {
    Changed,
}
impl EventEmitter<Navigate> for PathInput {}
impl EventEmitter<PathInputEvent> for PathInput {}
impl PathInput {
    pub fn new(
        reader: Arc<dyn DirectoryRead>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Directory path"));
        let subscription = cx.subscribe(&input, |this, input, event, cx| match event {
            InputEvent::Change => {
                let value = input.read(cx).value();
                if !this.navigation.draft_changed(value.as_ref()) {
                    return;
                }
                this.invalidate(false);
                cx.emit(PathInputEvent::Changed);
                cx.notify();
            }
            InputEvent::Blur => {
                this.invalidate(false);
                this.navigation.close_editor();
                cx.notify();
            }
            _ => {}
        });
        Self {
            input,
            window: window.window_handle(),
            focus: cx.focus_handle(),
            location: LocationState::new(reader),
            navigation: NavigationState::default(),
            completion: CompletionState::default(),
            scroll: UniformListScrollHandle::default(),
            _subscription: subscription,
        }
    }

    pub fn replace_reader(
        &mut self,
        reader: Option<Arc<dyn DirectoryRead>>,
        cx: &mut Context<Self>,
    ) {
        self.invalidate(true);
        self.location.replace_reader(reader);
        self.navigation.reset();
        cx.notify();
    }

    /// Called for every listing, including refresh and other navigation controls.
    pub fn begin_navigation_with_mode(&mut self, mode: NavigationMode, cx: &mut Context<Self>) {
        self.invalidate(true);
        self.navigation.begin(mode);
        cx.notify();
    }

    pub fn accept(
        &mut self,
        directory: Directory,
        home: Option<Directory>,
        cx: &mut Context<Self>,
    ) {
        self.invalidate(true);
        let value = match &directory {
            Directory::Local(path) => path.to_str().map(str::to_owned),
            Directory::Remote(path) => Some(path.clone()),
        };
        self.location.accept(directory, home);
        self.navigation.accepted(value.as_deref());
        let generation = self.completion.generation();
        let entity = cx.entity().downgrade();
        let handle = self.window;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    if this.completion.generation() != generation {
                        return;
                    }
                    if this.navigation.editing()
                        && let Some(value) = value
                    {
                        this.input
                            .update(cx, |input, cx| input.replace_all(value, window, cx));
                    } else if this.input.read(cx).focus_handle(cx).is_focused(window) {
                        window.focus(&this.focus, cx);
                    }
                });
            });
        });
        cx.notify();
    }

    pub fn navigation_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.navigation.failed(error);
        cx.notify();
    }

    /// The literal draft currently shown to the user.
    pub fn draft(&self, cx: &App) -> String {
        if self.navigation.editing() {
            self.input.read(cx).value().to_string()
        } else {
            self.location
                .directory()
                .and_then(Directory::text)
                .unwrap_or_default()
                .to_owned()
        }
    }
    pub fn visible_error(&self) -> Option<String> {
        self.navigation.visible_error()
    }
    pub fn directory(&self) -> Option<&Directory> {
        self.location.directory()
    }
    /// Resolve a form's current literal draft using the captured accepted location.
    pub fn resolved(&self, cx: &App) -> Result<Directory, nocterm_explorer::directory::PathError> {
        let base = self
            .location
            .directory()
            .ok_or(nocterm_explorer::directory::PathError::HomeUnavailable)?;
        resolve(&self.draft(cx), base, self.location.home())
    }
    fn invalidate(&mut self, cache: bool) {
        self.completion.invalidate(cache);
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.location.directory() else {
            return;
        };
        if self.navigation.navigating() {
            return;
        }
        let value = match directory {
            Directory::Local(path) => match path.to_str() {
                Some(path) => path.to_owned(),
                None => {
                    self.navigation.unrepresentable();
                    cx.notify();
                    return;
                }
            },
            Directory::Remote(path) => path.clone(),
        };
        self.invalidate(false);
        self.navigation.edit(&value);
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.synchronize_draft(cx);
        if self.navigation.navigating() {
            return;
        }
        let Some(base) = self.location.directory() else {
            return;
        };
        match resolve(
            self.input.read(cx).value().as_ref(),
            base,
            self.location.home(),
        ) {
            Ok(directory) => {
                let mode = if self.popup_visible() {
                    NavigationMode::KeepEditing
                } else {
                    NavigationMode::CloseEditor
                };
                self.begin_navigation_with_mode(mode, cx);
                cx.emit(Navigate { directory, mode });
            }
            Err(error) => {
                self.navigation.report_error(error.to_string());
                cx.notify();
            }
        }
        let _ = window;
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.invalidate(false);
        self.navigation.close_editor();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Native Change may arrive after the next keyboard action in the same update.
    /// Invalidate stale suggestions before that action can replace or submit a draft.
    fn synchronize_draft(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        if self.navigation.draft_changed(&value) {
            self.invalidate(false);
            cx.notify();
        }
    }

    fn complete(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.navigation.navigating() {
            return;
        }
        self.synchronize_draft(cx);
        if self.completion.continue_cycle() {
            self.advance(reverse, window, cx);
            return;
        }
        if self.completion.adjust_pending(reverse) {
            return;
        }
        let Some(base) = self.location.directory() else {
            return;
        };
        let query = match query(
            self.input.read(cx).value().as_ref(),
            base,
            self.location.home(),
        ) {
            Ok(query) => query,
            Err(error) => {
                self.navigation.report_error(error.to_string());
                cx.notify();
                return;
            }
        };
        self.navigation.clear_error();
        let query = match self.completion.cached(query) {
            Ok(()) => {
                self.advance(reverse, window, cx);
                return;
            }
            Err(query) => query,
        };
        let directory = query.directory.clone();
        let reader = self.location.reader();
        let (generation, cancel) = self.completion.begin(query, reverse);
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { read_candidates(directory, reader, cancel).await })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.completion.generation() != generation || !this.completion.loading() {
                    return;
                }
                match result {
                    Ok(entries) => {
                        if let Some(value) = this.completion.loaded(entries) {
                            this.apply(value, window, cx);
                        }
                    }
                    Err(error) => {
                        this.completion.failed();
                        this.navigation.report_error(error.to_string());
                    }
                }
                cx.notify();
            });
        });
        self.completion.own_task(task);
        cx.notify();
    }

    fn popup_visible(&self) -> bool {
        self.navigation.editing() && self.completion.visible()
    }

    fn arrow(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.synchronize_draft(cx);
        if !self.popup_visible() || self.navigation.navigating() {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
        if !self.completion.adjust_pending(reverse) {
            self.advance(reverse, window, cx);
        }
    }

    fn advance(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(value) = self.completion.advance(reverse) {
            self.apply(value, window, cx);
        }
        cx.notify();
    }
    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(value) = self.completion.choose(index) {
            self.apply(value, window, cx);
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }
    fn apply(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PathInputEvent::Changed);
        // Preserve native undo and ignore the Change caused by our own replacement.
        self.navigation.replaced(&value);
        self.input
            .update(cx, |input, cx| input.replace_all(value, window, cx));
        if let Some(index) = self.completion.selected() {
            self.scroll.scroll_to_item(index, ScrollStrategy::Top);
        }
    }
}

impl PathInput {
    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("explorer-path-editor")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_value(self.input.read(cx).value())
            .track_focus(&self.input.read(cx).focus_handle(cx))
            .key_context(KEY_CONTEXT)
            .capture_action(cx.listener(|this, _: &input::IndentInline, window, cx| {
                this.complete(false, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &input::OutdentInline, window, cx| {
                this.complete(true, window, cx)
            }))
            .capture_action(
                cx.listener(|this, _: &input::Enter, window, cx| this.submit(window, cx)),
            )
            .capture_action(
                cx.listener(|this, _: &input::Escape, window, cx| this.escape(window, cx)),
            )
            .capture_action(
                cx.listener(|this, _: &input::MoveUp, window, cx| this.arrow(true, window, cx)),
            )
            .capture_action(
                cx.listener(|this, _: &input::MoveDown, window, cx| this.arrow(false, window, cx)),
            )
            .child(path_metrics(
                Input::new(&self.input)
                    .small()
                    .readonly(self.navigation.navigating()),
            ))
    }
    fn render_display(
        &self,
        shown: SharedString,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        path_metrics(div())
            .id("explorer-path-display")
            .test_support()
            .w_full()
            .min_w_0()
            .border_1()
            .border_color(cx.theme().transparent)
            .rounded_sm()
            .when(self.focus.is_focused(window), |path| {
                path.bg(cx.theme().accent).border_color(cx.theme().primary)
            })
            .text_color(cx.theme().foreground)
            .overflow_hidden()
            .text_ellipsis()
            .when(!self.navigation.navigating(), |path| {
                path.cursor_text().hover(|path| path.bg(cx.theme().accent))
            })
            .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx)))
            .aria_value(shown.clone())
            .track_focus(&self.focus)
            .child(shown)
    }
}

impl Focusable for PathInput {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.navigation.editing() {
            self.input.read(cx).focus_handle(cx)
        } else {
            self.focus.clone()
        }
    }
}
impl Render for PathInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown: SharedString = self
            .location
            .directory()
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
                    if !this.navigation.editing()
                        && matches!(event.keystroke.key.as_str(), "enter" | "space")
                    {
                        cx.stop_propagation();
                        this.edit(window, cx);
                    }
                }),
            )
            .w_full()
            .min_w_0()
            .min_h_0()
            .gap_1()
            .when(self.location.directory().is_some(), |view| {
                if self.navigation.editing() {
                    view.child(self.render_editor(cx))
                } else {
                    view.child(self.render_display(shown, window, cx))
                }
            })
            .children(self.render_popup(window, cx))
            .when_some(self.navigation.visible_error(), |view, error| {
                view.child(
                    div()
                        .id("explorer-path-error")
                        .test_support()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .aria_label(error.clone())
                        .child(error),
                )
            })
    }
}

pub fn install_bindings(cx: &mut App) {
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
        KeyBinding::new("up", input::MoveUp, Some(KEY_CONTEXT)),
        KeyBinding::new("down", input::MoveDown, Some(KEY_CONTEXT)),
    ]);
}

async fn read_candidates(
    directory: Directory,
    reader: Option<Arc<dyn DirectoryRead>>,
    cancel: CancelFlag,
) -> Result<Vec<Candidate>, DirectoryError> {
    let reader = reader.ok_or(DirectoryError::Disconnected)?;
    Ok(reader.list(directory, cancel).await?.candidates)
}
