//! The name being typed for a layout or a group.

use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, SharedString, Subscription, Window,
    component::input::{InputEvent, InputState},
};
use nocterm_plugin_model::GroupId;
use nocterm_window_layout::{EditOp, LayoutSettings, PresetId, validate_name};

use crate::section::LayoutEditorSection;

/// What is being renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Renamed {
    Layout(PresetId),
    Group(GroupId),
}

/// A single-line input holding the new name of a layout or a group, with the reason the last
/// name was refused.
pub(crate) struct RenameDraft {
    renamed: Renamed,
    input: Entity<InputState>,
    error: Option<SharedString>,
    _subscription: Subscription,
}

impl RenameDraft {
    pub(crate) fn new(
        renamed: Renamed,
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
            renamed,
            input,
            error: None,
            _subscription: subscription,
        }
    }

    pub(crate) fn renamed(&self) -> &Renamed {
        &self.renamed
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

    /// The edit that gives the name typed, if it is accepted; otherwise the reason is kept
    /// beside the input. A layout's name is checked against the others here, a group's by the
    /// edit.
    pub(crate) fn checked(
        &mut self,
        settings: &LayoutSettings,
        cx: &gpui_kit::App,
    ) -> Option<EditOp> {
        let typed = self.input.read(cx).value().to_string();
        match &self.renamed {
            Renamed::Layout(preset) => match validate_name(settings, Some(preset), &typed) {
                Ok(name) => Some(EditOp::Rename { name }),
                Err(reason) => {
                    self.refuse(reason.to_string());
                    None
                }
            },
            Renamed::Group(group) => Some(EditOp::RenameGroup {
                group: group.clone(),
                name: typed,
            }),
        }
    }
}
