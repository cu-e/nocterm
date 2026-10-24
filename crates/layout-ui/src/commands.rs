//! How the parts of the editor reach the section that owns the state.

use gpui_kit::{App, Context, WeakEntity, Window};

use crate::section::LayoutEditorSection;

/// A handle for the click handlers of the editor's parts.
#[derive(Clone)]
pub(crate) struct Commands(WeakEntity<LayoutEditorSection>);

impl Commands {
    pub(crate) fn new(section: WeakEntity<LayoutEditorSection>) -> Self {
        Self(section)
    }

    /// Runs `command` on the section if it is still there.
    pub(crate) fn run(
        &self,
        window: &mut Window,
        cx: &mut App,
        command: impl FnOnce(&mut LayoutEditorSection, &mut Window, &mut Context<LayoutEditorSection>),
    ) {
        let _ = self
            .0
            .update(cx, |section, cx| command(section, window, cx));
    }
}
