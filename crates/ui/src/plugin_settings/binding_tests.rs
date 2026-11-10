//! The system source applies each edit to the section as it is when the write runs.

use gpui_kit::TestAppContext;
use nocterm_plugin_model::{FieldEdit, FieldError, SettingValue, SettingValues};
use nocterm_settings::{SettingsFile, SettingsSection};
use nocterm_store::SettingsConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{PluginSettingsError, PluginSettingsSource as _, SectionBinding, tests::SCHEMA};
use crate::{DesignTokens, TerminalSettings};

fn read(section: &TerminalSettings) -> SettingValues {
    let mut values = SCHEMA.defaults();
    values.insert("flag", SettingValue::Bool(section.cursor_blink));
    values.insert("name", SettingValue::Text(section.term.clone()));
    values.insert(
        "interval",
        SettingValue::Number(section.scrollback_lines as i32),
    );
    values
}

fn write(section: &mut TerminalSettings, key: &str, value: SettingValue) -> Result<(), FieldError> {
    match (key, value) {
        ("flag", SettingValue::Bool(on)) => section.cursor_blink = on,
        ("name", SettingValue::Text(text)) => section.term = text,
        ("interval", SettingValue::Number(n)) => section.scrollback_lines = n as u32,
        (key, _) => return Err(FieldError::UnknownKey(key.into())),
    }
    Ok(())
}

fn binding(
    config: SettingsConfig,
    cx: &mut gpui_kit::App,
) -> (
    SectionBinding<TerminalSettings>,
    nocterm_store::SettingsHandle,
) {
    gpui_kit::init(cx);
    let handle = config.install(cx);
    crate::init(DesignTokens::builtin(), handle.clone(), cx);
    (
        SectionBinding::new(handle.clone(), &SCHEMA, read, write),
        handle,
    )
}

#[gpui_kit::test]
fn two_quick_edits_both_land(cx: &mut TestAppContext) {
    let (binding, handle) =
        cx.update(|cx| binding(SettingsConfig::in_memory(Default::default()), cx));
    let (first, second) = cx.update(|cx| {
        (
            binding.write("flag", FieldEdit::Set(SettingValue::Bool(false)), cx),
            binding.write(
                "name",
                FieldEdit::Set(SettingValue::Text("vt100".into())),
                cx,
            ),
        )
    });
    cx.run_until_parked();
    drop((first, second));
    cx.update(|cx| {
        let section = handle.setting::<TerminalSettings>(cx);
        assert!(!section.cursor_blink);
        assert_eq!(section.term, "vt100");
    });
}

#[gpui_kit::test]
async fn a_refused_edit_comes_back_as_the_schema_error_and_changes_nothing(
    cx: &mut TestAppContext,
) {
    let (binding, handle) =
        cx.update(|cx| binding(SettingsConfig::in_memory(Default::default()), cx));
    let before = cx.update(|cx| handle.setting::<TerminalSettings>(cx).clone());
    let task =
        cx.update(|cx| binding.write("interval", FieldEdit::Set(SettingValue::Number(99)), cx));
    cx.run_until_parked();
    let error = task.await.unwrap_err();
    assert!(matches!(
        error,
        PluginSettingsError::Field(FieldError::OutOfRange { .. })
    ));
    assert_eq!(error.to_string(), "Enter a whole number from 1 to 10.");
    cx.update(|cx| assert_eq!(*handle.setting::<TerminalSettings>(cx), before));
}

#[gpui_kit::test]
async fn an_unstorable_value_is_reported_as_a_failed_write(cx: &mut TestAppContext) {
    let blocked = tempfile::tempdir().unwrap();
    let config = SettingsConfig::new(
        Default::default(),
        nocterm_settings::SettingsFile::new(blocked.path()),
    );
    let (binding, handle) = cx.update(|cx| binding(config, cx));
    let task = cx.update(|cx| binding.write("flag", FieldEdit::Set(SettingValue::Bool(false)), cx));
    cx.run_until_parked();
    let error = task.await.unwrap_err();
    assert!(matches!(error, PluginSettingsError::Write(_)), "{error}");
    assert!(error.to_string().starts_with("Could not save the setting"));
    cx.update(|cx| assert!(handle.setting::<TerminalSettings>(cx).cursor_blink));
}

/// Kept in a module: the settings reference lists top-level sections only, and this one
/// exists for the tests.
mod stored {
    use super::*;

    /// A section with one stored value for each of the `metrics`, `name` and `interval` fields of
    /// the test schema, so the durable store can be driven without the terminal's own rules.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(default, deny_unknown_fields)]
    pub(super) struct Local {
        pub(super) metrics: Vec<String>,
        pub(super) name: String,
        pub(super) interval: i32,
    }

    impl Default for Local {
        fn default() -> Self {
            Self {
                metrics: vec!["cpu".into()],
                name: String::new(),
                interval: 5,
            }
        }
    }

    impl SettingsSection for Local {
        const KEY: &'static str = "binding_test_local";
    }
}
use stored::Local;

fn read_local(section: &Local) -> SettingValues {
    let mut values = SCHEMA.defaults();
    values.insert("metrics", SettingValue::Choices(section.metrics.clone()));
    values.insert("name", SettingValue::Text(section.name.clone()));
    values.insert("interval", SettingValue::Number(section.interval));
    values
}

fn write_local(section: &mut Local, key: &str, value: SettingValue) -> Result<(), FieldError> {
    match (key, value) {
        ("metrics", SettingValue::Choices(chosen)) => section.metrics = chosen,
        ("name", SettingValue::Text(text)) => section.name = text,
        ("interval", SettingValue::Number(number)) => section.interval = number,
        (key, _) => return Err(FieldError::UnknownKey(key.into())),
    }
    Ok(())
}

/// A binding over a section stored in a real settings file.
fn durable(
    section: Local,
    dir: &std::path::Path,
    cx: &mut gpui_kit::App,
) -> (SectionBinding<Local>, nocterm_store::SettingsHandle) {
    gpui_kit::init(cx);
    let handle = SettingsConfig::new(
        Default::default(),
        SettingsFile::new(dir.join("settings.toml")),
    )
    .with(section)
    .install(cx);
    let binding = SectionBinding::new(handle.clone(), &SCHEMA, read_local, write_local);
    (binding, handle)
}

fn toggle(option: &str) -> FieldEdit {
    FieldEdit::Toggle {
        option: option.into(),
        on: true,
    }
}

#[gpui_kit::test]
async fn two_quick_toggles_of_one_list_both_land_in_the_durable_store(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (binding, handle) = cx.update(|cx| durable(Local::default(), dir.path(), cx));
    let (first, second) = cx.update(|cx| {
        (
            binding.write("metrics", toggle("memory"), cx),
            binding.write("metrics", toggle("swap"), cx),
        )
    });
    first.await.unwrap();
    second.await.unwrap();
    cx.update(|cx| {
        assert_eq!(
            handle.setting::<Local>(cx).metrics,
            ["cpu", "memory", "swap"]
        );
    });
}

#[gpui_kit::test]
async fn two_quick_steps_both_land_in_the_durable_store(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let start = Local {
        interval: 1,
        ..Local::default()
    };
    let (binding, handle) = cx.update(|cx| durable(start, dir.path(), cx));
    let (first, second) = cx.update(|cx| {
        (
            binding.write("interval", FieldEdit::Step { up: true }, cx),
            binding.write("interval", FieldEdit::Step { up: true }, cx),
        )
    });
    first.await.unwrap();
    second.await.unwrap();
    cx.update(|cx| assert_eq!(handle.setting::<Local>(cx).interval, 7));
}

#[gpui_kit::test]
fn stored_text_is_sanitized_and_cut_to_its_field_length_when_read(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let dirty = Local {
        name: format!("Os\u{202E}lo{}", "x".repeat(300)),
        ..Local::default()
    };
    let (binding, _handle) = cx.update(|cx| durable(dirty, dir.path(), cx));
    let values = cx.update(|cx| binding.values(cx));
    assert_eq!(
        values.get("name"),
        Some(&SettingValue::Text("Osloxxxx".into()))
    );
}
