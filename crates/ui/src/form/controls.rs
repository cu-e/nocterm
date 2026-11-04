//! Common control presentation receives the owner's event handlers.
use gpui_kit::{
    AnyElement, App, ClickEvent, Window,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    prelude::*,
};

pub fn choices<T: Copy + PartialEq, F>(
    id: &'static str,
    current: T,
    options: &[(&'static str, T)],
    mut on_choose: impl FnMut(T) -> F,
) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    h_flex()
        .gap_1()
        .children(options.iter().enumerate().map(|(index, (label, value))| {
            Button::new((id, index))
                .small()
                .ghost()
                .label(*label)
                .selected(current == *value)
                .on_click(on_choose(*value))
        }))
        .into_any_element()
}
