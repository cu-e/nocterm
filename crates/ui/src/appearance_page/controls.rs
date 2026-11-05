use super::AppearanceSettingsPage;
use crate::form;
use gpui_kit::{
    AnyElement, App, Context, SharedString,
    component::{Disableable as _, switch::Switch},
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
        .on_change(cx.listener(move |this, checked: &bool, _, cx| {
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
    form::choices(id, current, options, |value| {
        cx.listener(move |this, _, _, cx| this.save(move |settings| set(settings, value), cx))
    })
}

pub(crate) fn short_row(
    view: &AppearanceSettingsPage,
    key: &str,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    form::short_field(view.field(key), key, label, description, cx)
}
