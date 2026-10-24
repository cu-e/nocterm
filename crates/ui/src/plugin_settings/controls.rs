//! The controls of the settings form, one per field kind.

use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _,
    component::{
        Selectable as _, Sizable as _,
        button::Button,
        checkbox::Checkbox,
        h_flex,
        input::{Input, InputState},
        switch::Switch,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_plugin_model::{ChoiceOption, FieldEdit, FieldSpec, PluginId, SettingValue};

use super::PluginSettingsForm;
use crate::{PickerOption, form, open_choice_picker};

/// Choices up to this many are shown as a row of buttons; more open a searchable list.
const SEGMENTED_MAX: usize = 4;

/// The element id of a control, unique across plugins and fields.
pub(super) fn control_id(plugin: &PluginId, key: &str, part: &str) -> SharedString {
    format!("plugin-setting:{plugin}:{key}:{part}").into()
}

fn help(field: &FieldSpec) -> SharedString {
    field.help.as_deref().unwrap_or_default().to_string().into()
}

pub(super) fn toggle_row(
    field: &FieldSpec,
    id: SharedString,
    on: bool,
    cx: &mut Context<PluginSettingsForm>,
) -> AnyElement {
    let key = field.key.to_string();
    let switch =
        Switch::new(id)
            .checked(on)
            .on_click(cx.listener(move |form, checked: &bool, _, cx| {
                form.edit(&key, FieldEdit::Set(SettingValue::Bool(*checked)), cx);
            }));
    form::row(field.label.to_string(), help(field), switch, cx)
}

pub(super) fn choice_row(
    field: &FieldSpec,
    options: &[ChoiceOption],
    current: &str,
    plugin: &PluginId,
    cx: &mut Context<PluginSettingsForm>,
) -> AnyElement {
    let key = field.key.to_string();
    let control = if options.len() <= SEGMENTED_MAX {
        let mut row = h_flex().gap_1();
        for option in options {
            let (chosen, option_key) = (option.key.to_string(), key.clone());
            row = row.child(
                Button::new(control_id(plugin, &key, &option.key))
                    .small()
                    .outline()
                    .label(option.label.to_string())
                    .selected(option.key == current)
                    .on_click(cx.listener(move |form, _, _, cx| {
                        let value = SettingValue::Choice(chosen.clone());
                        form.edit(&option_key, FieldEdit::Set(value), cx);
                    })),
            );
        }
        row.into_any_element()
    } else {
        let label = options
            .iter()
            .find(|option| option.key == current)
            .map_or(current, |option| &option.label)
            .to_string();
        let picker: Vec<(String, String, bool)> = options
            .iter()
            .map(|option| {
                (
                    option.key.to_string(),
                    option.label.to_string(),
                    option.key == current,
                )
            })
            .collect();
        Button::new(control_id(plugin, &key, "choose"))
            .small()
            .outline()
            .label(label)
            .on_click(cx.listener(move |_, _, window, cx| {
                let form = cx.entity();
                let key = key.clone();
                let options = picker
                    .iter()
                    .map(|(value, label, checked)| {
                        PickerOption::new(label.clone(), *checked, value.clone())
                    })
                    .collect();
                open_choice_picker(
                    "Choose an option…",
                    options,
                    move |value, _, cx| {
                        form.update(cx, |form, cx| {
                            form.edit(&key, FieldEdit::Set(SettingValue::Choice(value)), cx);
                        });
                    },
                    window,
                    cx,
                );
            }))
            .into_any_element()
    };
    form::row(field.label.to_string(), help(field), control, cx)
}

pub(super) fn multi_row(
    field: &FieldSpec,
    options: &[ChoiceOption],
    chosen: &[String],
    plugin: &PluginId,
    cx: &mut Context<PluginSettingsForm>,
) -> AnyElement {
    let mut rows = vec![form::row(field.label.to_string(), help(field), div(), cx)];
    for option in options {
        let (key, option_key) = (field.key.to_string(), option.key.to_string());
        let on = chosen.iter().any(|item| *item == option.key);
        rows.push(
            h_flex()
                .w_full()
                .pb_2()
                .pl_4()
                .child(
                    Checkbox::new(control_id(plugin, &field.key, &option.key))
                        .label(option.label.to_string())
                        .checked(on)
                        .on_click(cx.listener(move |form, checked: &bool, _, cx| {
                            let edit = FieldEdit::Toggle {
                                option: option_key.clone(),
                                on: *checked,
                            };
                            form.edit(&key, edit, cx);
                        })),
                )
                .into_any_element(),
        );
    }
    v_flex().w_full().children(rows).into_any_element()
}

pub(super) fn number_row(
    field: &FieldSpec,
    current: i32,
    unit: Option<&str>,
    plugin: &PluginId,
    cx: &mut Context<PluginSettingsForm>,
) -> AnyElement {
    let (down_key, up_key) = (field.key.to_string(), field.key.to_string());
    let shown = SharedString::from(current.to_string());
    let control = h_flex()
        .gap_1()
        .items_center()
        .child(
            Button::new(control_id(plugin, &field.key, "down"))
                .small()
                .outline()
                .label("−")
                .on_click(cx.listener(move |form, _, _, cx| form.nudge(&down_key, false, cx))),
        )
        .child(
            div()
                .id(control_id(plugin, &field.key, "value"))
                .aria_label(shown.clone())
                .test_support()
                .min_w_12()
                .text_center()
                .text_sm()
                .child(match unit {
                    Some(unit) => format!("{shown} {unit}"),
                    None => shown.to_string(),
                }),
        )
        .child(
            Button::new(control_id(plugin, &field.key, "up"))
                .small()
                .outline()
                .label("+")
                .on_click(cx.listener(move |form, _, _, cx| form.nudge(&up_key, true, cx))),
        );
    form::row(field.label.to_string(), help(field), control, cx)
}

pub(super) fn text_row(
    field: &FieldSpec,
    id: SharedString,
    input: Option<&Entity<InputState>>,
    cx: &mut Context<PluginSettingsForm>,
) -> AnyElement {
    let control = match input {
        Some(input) => div()
            .id(id)
            .test_support()
            .w_full()
            .child(Input::new(input).small())
            .into_any_element(),
        None => div().into_any_element(),
    };
    form::stacked_row(field.label.to_string(), help(field), control, None, cx)
}

/// A field whose stored value has the wrong shape for its kind; shown, never edited.
pub(super) fn mismatch_row(field: &FieldSpec, cx: &mut Context<PluginSettingsForm>) -> AnyElement {
    form::row(
        field.label.to_string(),
        "This setting holds a value of the wrong kind.",
        div(),
        cx,
    )
}
