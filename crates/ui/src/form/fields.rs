//! Stateless presentation of a checked text field; membership stays with its page.
use crate::{SettingsTextField, form};
use gpui_kit::{
    AnyElement, App, SharedString, TestSupportExt as _, component::v_flex, prelude::*, rems,
};

pub fn text_field(
    field: &SettingsTextField,
    key: &str,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    gpui_kit::div()
        .id(SharedString::from(format!("settings-field-{key}")))
        .test_support()
        .child(form::stacked_row(
            label,
            description,
            field.input(),
            field.error(cx),
            cx,
        ))
        .into_any_element()
}

pub fn short_field(
    field: &SettingsTextField,
    key: &str,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
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
