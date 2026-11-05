use super::AgentSettingsPage;
use gpui_kit::{
    AnyElement, App, Context, SharedString,
    component::{Disableable as _, switch::Switch},
};
use nocterm_settings::SettingsSection;
use nocterm_ui::form;
pub(crate) fn toggle<S: SettingsSection>(
    id: impl Into<gpui_kit::ElementId>,
    checked: bool,
    disabled: bool,
    set: fn(&mut S, bool),
    cx: &mut Context<AgentSettingsPage>,
) -> Switch {
    Switch::new(id)
        .checked(checked)
        .disabled(disabled)
        .on_change(cx.listener(move |this, checked: &bool, _, cx| {
            let checked = *checked;
            this.save(move |settings| set(settings, checked), cx);
        }))
}

pub(crate) fn text_row(
    view: &AgentSettingsPage,
    key: &str,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    form::text_field(view.field(key), key, label, description, cx)
}
