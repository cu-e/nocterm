//! The name being typed for a layout.

use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, SharedString, Subscription, Window,
    component::input::{InputEvent, InputState},
};
use nocterm_window_layout::{LayoutSettings, PresetId, validate_name};

use crate::section::LayoutEditorSection;

/// A single-line input holding the new name of a layout, with the reason the last name was
/// refused.
pub(crate) struct RenameDraft {
    preset: PresetId,
    input: Entity<InputState>,
    error: Option<SharedString>,
    _subscription: Subscription,
}

impl RenameDraft {
    pub(crate) fn new(
        preset: PresetId,
        name: String,
        window: &mut Window,
        cx: &mut Context<LayoutEditorSection>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |section, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => section.commit_rename(window, cx),
                    InputEvent::Change => section.clear_rename_error(cx),
                    _ => {}
                },
            );
        window.focus(&input.read(cx).focus_handle(cx), cx);
        Self {
            preset,
            input,
            error: None,
            _subscription: subscription,
        }
    }

    pub(crate) fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn refuse(&mut self, reason: impl Into<SharedString>) {
        self.error = Some(reason.into());
    }

    pub(crate) fn clear_error(&mut self) {
        self.error = None;
    }

    /// The name typed, if the settings accept it for the layout; otherwise the reason is kept
    /// beside the input.
    pub(crate) fn checked(
        &mut self,
        settings: &LayoutSettings,
        cx: &gpui_kit::App,
    ) -> Option<String> {
        let typed = self.input.read(cx).value().to_string();
        match validate_name(settings, Some(&self.preset), &typed) {
            Ok(name) => Some(name),
            Err(reason) => {
                self.refuse(reason.to_string());
                None
            }
        }
    }
}
