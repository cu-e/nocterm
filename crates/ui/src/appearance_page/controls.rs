use super::AppearanceSettingsPage;
use crate::form;
use gpui_kit::{
    AnyElement, App, Context, SharedString, TestSupportExt as _,
    component::{
        Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        switch::Switch,
        v_flex,
    },
    prelude::*,
    rems,
};
use nocterm_settings::SettingsSection;
pub(crate) fn toggle<S: SettingsSection>(
    id: impl Into<gpui_kit::ElementId>,
    checked: bool,
    disabled: bool,
    set: fn(&mut S, bool),
    cx: &mut Context<AppearanceSettingsPage>,
) -> Switch {
    Switch::new(id)
        .checked(checked)
        .disabled(disabled)
        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
            let checked = *checked;
            this.save(move |settings| set(settings, checked), cx);
        }))
}
pub(crate) fn choices<S: SettingsSection, T: Copy + PartialEq + 'static>(
    id: &'static str,
    current: T,
    options: &[(&'static str, T)],
    set: fn(&mut S, T),
    cx: &mut Context<AppearanceSettingsPage>,
) -> AnyElement {
    h_flex()
        .gap_1()
        .children(options.iter().enumerate().map(|(index, (label, value))| {
            let value = *value;
            Button::new((id, index))
                .small()
                .ghost()
                .label(*label)
                .selected(current == value)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.save(move |settings| set(settings, value), cx)
                }))
        }))
        .into_any_element()
}

pub(crate) fn short_row(
    view: &AppearanceSettingsPage,
    key: &str,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let field = view.field(key);
    let row = form::row(
        label,
        description,
        gpui_kit::div().w(rems(10.)).child(field.input()),
        cx,
    );
    match field.error(cx) {
        Some(error) => v_flex()
            .id(SharedString::from(format!("settings-error-{key}")))
            .aria_label(error.clone())
            .test_support()
            .child(row)
            .child(form::error_text(error, cx))
            .into_any_element(),
        None => row,
    }
}
