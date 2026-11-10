//! A captured filesystem capability and real directory validation, independent of a form.
mod state;
#[cfg(test)]
mod tests;
use super::{
    Directory, DirectoryNavigation, Navigate, NavigationAction, NavigationMode, PathInput,
    PathInputEvent,
};
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Window,
    component::{ActiveTheme as _, v_flex},
    div,
    prelude::*,
};
use nocterm_explorer::directory::{DirectoryError, DirectoryRead};
use state::PickerState;
use std::sync::Arc;

pub enum DirectoryPickerEvent {
    Changed,
    Validated(Directory),
    Failed(DirectoryError),
}
/// Read-only navigation. No create, rename or other mutation capability is held.
pub struct DirectoryPicker {
    path: Entity<PathInput>,
    state: PickerState,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<DirectoryPickerEvent> for DirectoryPicker {}
impl DirectoryPicker {
    pub fn new(
        reader: Arc<dyn DirectoryRead>,
        initial: Option<Directory>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let path = cx.new(|cx| PathInput::new(reader.clone(), window, cx));
        if let Some(directory) = initial.clone() {
            path.update(cx, |path, cx| path.accept(directory, None, cx));
        }
        let navigation = cx.subscribe(&path, |this, _, event: &Navigate, cx| {
            this.load(Some(event.directory.clone()), event.mode, cx);
        });
        let changes = cx.subscribe(&path, |this, _, _: &PathInputEvent, cx| {
            this.state.invalidate();
            cx.emit(DirectoryPickerEvent::Changed);
            cx.notify();
        });
        let mut picker = Self {
            path,
            state: PickerState::new(reader),
            _subscriptions: vec![navigation, changes],
        };
        picker.load(initial, NavigationMode::CloseEditor, cx);
        picker
    }
    pub fn replace_reader(
        &mut self,
        reader: Arc<dyn DirectoryRead>,
        initial: Option<Directory>,
        cx: &mut Context<Self>,
    ) {
        self.path
            .update(cx, |path, cx| path.replace_reader(Some(reader.clone()), cx));
        self.state.replace_reader(reader);
        self.load(initial, NavigationMode::CloseEditor, cx);
        cx.emit(DirectoryPickerEvent::Changed);
    }
    /// Supply a literal editing base when the source cannot resolve its home.
    /// This does not mark the directory valid; validation still reads the source.
    pub fn set_fallback(&mut self, directory: Directory, cx: &mut Context<Self>) {
        if self.path.read(cx).directory().is_none() {
            self.path
                .update(cx, |path, cx| path.accept(directory, None, cx));
        }
    }
    pub fn draft(&self, cx: &App) -> String {
        self.path.read(cx).draft(cx)
    }
    pub fn is_valid(&self, cx: &App) -> bool {
        !self.state.loading()
            && self
                .state
                .accepted()
                .is_some_and(|directory| directory.text() == Some(self.draft(cx).as_str()))
    }
    pub fn value(&self, cx: &App) -> Option<Directory> {
        self.is_valid(cx)
            .then(|| self.state.accepted().cloned())
            .flatten()
    }
    /// Admits a real listing; emits Validated only when it succeeds for this exact draft/source.
    pub fn validate(&mut self, cx: &mut Context<Self>) {
        match self.path.read(cx).resolved(cx) {
            Ok(directory) => self.load(Some(directory), NavigationMode::CloseEditor, cx),
            Err(error) => {
                self.path
                    .update(cx, |path, cx| path.navigation_failed(error.to_string(), cx));
                self.state.invalidate();
                cx.emit(DirectoryPickerEvent::Failed(DirectoryError::Path(error)));
            }
        }
    }
    fn action(&mut self, action: NavigationAction, cx: &mut Context<Self>) {
        let directory = match action {
            NavigationAction::Home => None,
            NavigationAction::Up => self.path.read(cx).directory().and_then(Directory::parent),
            NavigationAction::Refresh => self.path.read(cx).directory().cloned(),
        };
        self.load(directory, NavigationMode::CloseEditor, cx);
    }
    fn load(&mut self, directory: Option<Directory>, mode: NavigationMode, cx: &mut Context<Self>) {
        let (generation, cancel, reader) = self.state.begin();
        self.path
            .update(cx, |path, cx| path.begin_navigation_with_mode(mode, cx));
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let home = reader.home().await;
                    let directory = match directory {
                        Some(directory) => directory,
                        None => home.clone()?,
                    };
                    let listing = reader.list(directory, cancel).await?;
                    Ok::<_, DirectoryError>((listing.directory, home.ok()))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.is_current(generation) {
                    return;
                }
                match result {
                    Ok((directory, home)) => {
                        this.state.accept(directory.clone());
                        this.path
                            .update(cx, |path, cx| path.accept(directory.clone(), home, cx));
                        cx.emit(DirectoryPickerEvent::Validated(directory));
                    }
                    Err(error) => {
                        this.state.fail(error.clone());
                        this.path
                            .update(cx, |path, cx| path.navigation_failed(error.to_string(), cx));
                        cx.emit(DirectoryPickerEvent::Failed(error));
                    }
                }
                cx.notify();
            });
        });
        self.state.own_task(task);
        cx.notify();
    }
}
impl Focusable for DirectoryPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.path.read(cx).focus_handle(cx)
    }
}
impl Render for DirectoryPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let picker = cx.entity().downgrade();
        let can_up = self
            .path
            .read(cx)
            .directory()
            .and_then(Directory::parent)
            .is_some();
        v_flex()
            .id("directory-picker")
            .w_full()
            .min_w_0()
            .gap_1()
            .child(DirectoryNavigation::new(
                "directory",
                !self.state.loading(),
                can_up,
                move |action, _, cx| {
                    let _ = picker.update(cx, |picker, cx| picker.action(action, cx));
                },
            ))
            .child(self.path.clone())
            .when(self.state.loading(), |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Reading directory…"),
                )
            })
            .when_some(
                self.state
                    .error()
                    .filter(|_| self.path.read(cx).visible_error().is_none()),
                |view, error| {
                    view.child(
                        div()
                            .id("directory-picker-error")
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(error.to_string()),
                    )
                },
            )
    }
}
