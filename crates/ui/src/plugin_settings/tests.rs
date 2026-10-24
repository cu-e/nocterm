//! The form draws a schema, sends edits through its source and shows what the source holds.

use std::{borrow::Cow, rc::Rc, sync::LazyLock};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions,
    test::TestWindowExt as _,
};
use nocterm_plugin_model::{
    ChoiceOption, ContributionId, FieldEdit, FieldGroup, FieldKind, FieldSpec, PluginId,
    SettingValue, SettingsSchema,
};
use nocterm_store::SettingsConfig;

use super::{
    FakePluginSettings, FormFrame, FormScope, PluginSettingsError, PluginSettingsForm,
    PluginSettingsSource,
};
use crate::DesignTokens;

fn options(keys: &[&'static str]) -> Cow<'static, [ChoiceOption]> {
    keys.iter()
        .map(|key| ChoiceOption {
            key: Cow::Borrowed(*key),
            label: Cow::Borrowed(*key),
            help: None,
        })
        .collect::<Vec<_>>()
        .into()
}

fn field(key: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec {
        key: Cow::Borrowed(key),
        label: Cow::Borrowed(key),
        help: None,
        kind,
    }
}

pub(super) static SCHEMA: LazyLock<SettingsSchema> = LazyLock::new(|| SettingsSchema {
    groups: vec![
        FieldGroup {
            title: Cow::Borrowed("General"),
            contribution: None,
            fields: vec![
                field("flag", FieldKind::Toggle { default: false }),
                field(
                    "mode",
                    FieldKind::Choice {
                        options: options(&["fast", "slow"]),
                        default: Cow::Borrowed("fast"),
                    },
                ),
                field(
                    "metrics",
                    FieldKind::MultiChoice {
                        options: options(&["cpu", "memory", "swap"]),
                        default: vec![Cow::Borrowed("cpu")].into(),
                        ordered: false,
                        max: 3,
                    },
                ),
                field(
                    "interval",
                    FieldKind::Number {
                        min: 1,
                        max: 10,
                        step: 3,
                        default: 5,
                        unit: Some(Cow::Borrowed("s")),
                    },
                ),
                field(
                    "name",
                    FieldKind::Text {
                        default: Cow::Borrowed(""),
                        max_len: 8,
                        placeholder: Some(Cow::Borrowed("a name")),
                    },
                ),
            ]
            .into(),
        },
        FieldGroup {
            title: Cow::Borrowed("Extra"),
            contribution: Some(ContributionId::new("extra")),
            fields: vec![field("more", FieldKind::Toggle { default: true })].into(),
        },
    ]
    .into(),
});

pub(super) struct Fixture {
    pub window: AnyWindowHandle,
    pub form: Entity<PluginSettingsForm>,
    pub source: FakePluginSettings,
}

pub(super) fn open(scope: FormScope, frame: FormFrame, cx: &mut TestAppContext) -> Fixture {
    let fixture = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init(
            DesignTokens::builtin(),
            SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        let source = FakePluginSettings::new(&SCHEMA, cx);
        let shared: Rc<dyn PluginSettingsSource> = Rc::new(source.clone());
        let (window, form) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                PluginSettingsForm::new(
                    shared,
                    PluginId::new("test"),
                    &SCHEMA,
                    scope,
                    frame,
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        Fixture {
            window,
            form,
            source,
        }
    });
    cx.update_window(fixture.window, |_, window, _| window.activate_window())
        .unwrap();
    cx.simulate_window_resize(
        fixture.window,
        gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(1400.)),
    );
    cx.run_until_parked();
    fixture
}

pub(super) fn id(key: &str, part: &str) -> String {
    format!("plugin-setting:test:{key}:{part}")
}

pub(super) fn click(fixture: &Fixture, id: String, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(gpui_kit::SharedString::from(id), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

pub(super) fn label_of(fixture: &Fixture, id: String, cx: &mut TestAppContext) -> Option<String> {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(gpui_kit::SharedString::from(id))
            .and_then(|found| found.label().map(str::to_owned))
    })
    .unwrap()
}

fn present(fixture: &Fixture, id: String, cx: &mut TestAppContext) -> bool {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(gpui_kit::SharedString::from(id)).is_some()
    })
    .unwrap()
}

fn value(fixture: &Fixture, key: &str, cx: &mut TestAppContext) -> SettingValue {
    cx.update(|cx| fixture.source.values(cx).get(key).cloned().unwrap())
}

#[gpui_kit::test]
fn a_toggle_sends_its_new_state(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    click(&fixture, id("flag", ""), cx);
    assert_eq!(value(&fixture, "flag", cx), SettingValue::Bool(true));
    assert_eq!(
        fixture.source.edits(),
        vec![("flag".into(), FieldEdit::Set(SettingValue::Bool(true)))]
    );
}

#[gpui_kit::test]
fn a_choice_button_selects_its_option(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    click(&fixture, id("mode", "slow"), cx);
    assert_eq!(
        value(&fixture, "mode", cx),
        SettingValue::Choice("slow".into())
    );
}

#[gpui_kit::test]
fn a_multi_choice_option_is_toggled_on_the_current_value(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    click(&fixture, id("metrics", "memory"), cx);
    click(&fixture, id("metrics", "cpu"), cx);
    assert_eq!(
        value(&fixture, "metrics", cx),
        SettingValue::Choices(vec!["memory".into()])
    );
    assert_eq!(
        fixture.source.edits()[1].1,
        FieldEdit::Toggle {
            option: "cpu".into(),
            on: false
        }
    );
}

#[gpui_kit::test]
fn a_number_steps_and_stops_at_its_bounds(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    assert_eq!(
        label_of(&fixture, id("interval", "value"), cx).as_deref(),
        Some("5")
    );
    click(&fixture, id("interval", "up"), cx);
    click(&fixture, id("interval", "up"), cx);
    assert_eq!(value(&fixture, "interval", cx), SettingValue::Number(10));
    click(&fixture, id("interval", "up"), cx);
    assert_eq!(fixture.source.edits().len(), 2, "no edit past the bound");
    click(&fixture, id("interval", "down"), cx);
    assert_eq!(value(&fixture, "interval", cx), SettingValue::Number(7));
}

#[gpui_kit::test]
fn a_change_made_elsewhere_is_shown(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    cx.update(|cx| fixture.source.set("interval", SettingValue::Number(9), cx));
    cx.run_until_parked();
    assert_eq!(
        label_of(&fixture, id("interval", "value"), cx).as_deref(),
        Some("9")
    );
}

#[gpui_kit::test]
fn a_refused_edit_shows_why_under_its_field(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    fixture.source.refuse_next(PluginSettingsError::Field(
        nocterm_plugin_model::FieldError::OutOfRange {
            key: "flag".into(),
            min: 1,
            max: 10,
        },
    ));
    click(&fixture, id("flag", ""), cx);
    assert_eq!(
        label_of(&fixture, id("flag", "error"), cx).as_deref(),
        Some("Enter a whole number from 1 to 10.")
    );
    assert_eq!(value(&fixture, "flag", cx), SettingValue::Bool(false));
    click(&fixture, id("flag", ""), cx);
    assert_eq!(label_of(&fixture, id("flag", "error"), cx), None);
}

#[gpui_kit::test]
fn a_contribution_scope_shows_only_its_own_groups(cx: &mut TestAppContext) {
    let fixture = open(
        FormScope::Contribution(ContributionId::new("extra")),
        FormFrame::Host,
        cx,
    );
    assert!(present(&fixture, id("more", ""), cx));
    assert!(!present(&fixture, id("flag", ""), cx));
    click(&fixture, id("more", ""), cx);
    assert_eq!(value(&fixture, "more", cx), SettingValue::Bool(false));
}

#[gpui_kit::test]
fn a_contribution_without_settings_says_so(cx: &mut TestAppContext) {
    let fixture = open(
        FormScope::Contribution(ContributionId::new("none")),
        FormFrame::Host,
        cx,
    );
    assert_eq!(
        label_of(&fixture, "plugin-settings-empty:test".into(), cx).as_deref(),
        Some("This item has no settings")
    );
}

#[gpui_kit::test]
fn a_plugin_frame_names_the_plugin(cx: &mut TestAppContext) {
    let fixture = open(
        FormScope::Plugin,
        FormFrame::Plugin {
            name: "Weather".into(),
        },
        cx,
    );
    assert_eq!(
        label_of(&fixture, "plugin-settings-frame:test".into(), cx).as_deref(),
        Some("Plugin · Weather")
    );
}
