//! Small pieces the parts of the editor share.

use gpui_kit::{
    App, ElementId, SharedString, Window,
    component::{Disableable as _, Sizable as _, button::Button},
};

/// A small outlined button. A disabled one ignores clicks.
pub(crate) fn action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .small()
        .outline()
        .label(label)
        .disabled(!enabled)
        .on_click(move |_, window, cx| on_click(window, cx))
}
