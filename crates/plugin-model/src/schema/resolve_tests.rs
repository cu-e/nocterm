use super::{fixtures::*, *};
use alloc::{borrow::Cow, string::ToString, vec};

fn values(entries: &[(&str, SettingValue)]) -> SettingValues {
    let mut values = SettingValues::default();
    for (key, value) in entries {
        values.insert(*key, value.clone());
    }
    values
}

fn chosen(list: &[&str]) -> SettingValue {
    SettingValue::Choices(list.iter().map(ToString::to_string).collect())
}

#[test]
fn an_empty_document_resolves_to_the_defaults_without_issues() {
    let schema = sample();
    let (resolved, issues) = resolve(&schema, &SettingValues::default());
    assert_eq!(resolved, schema.defaults());
    assert!(issues.is_empty());
}

#[test]
fn valid_stored_values_are_kept() {
    let schema = sample();
    let stored = values(&[
        ("on", SettingValue::Bool(false)),
        ("mode", SettingValue::Choice("slow".into())),
        ("tags", chosen(&["b", "a"])),
        ("count", SettingValue::Number(8)),
        ("city", SettingValue::Text("Rome".into())),
    ]);
    let (resolved, issues) = resolve(&schema, &stored);
    assert_eq!(resolved, stored);
    assert!(issues.is_empty());
}

#[test]
fn unknown_keys_are_dropped() {
    let schema = sample();
    let stored = values(&[
        ("gone", SettingValue::Bool(true)),
        ("on", SettingValue::Bool(false)),
    ]);
    let (resolved, issues) = resolve(&schema, &stored);
    assert!(resolved.get("gone").is_none());
    assert_eq!(resolved.get("on"), Some(&SettingValue::Bool(false)));
    assert!(issues.is_empty());
}

#[test]
fn a_value_of_the_wrong_kind_falls_back_to_the_default_with_an_issue() {
    let schema = sample();
    let stored = values(&[
        ("on", SettingValue::Number(1)),
        ("city", SettingValue::Bool(true)),
    ]);
    let (resolved, issues) = resolve(&schema, &stored);
    assert_eq!(resolved.get("on"), Some(&SettingValue::Bool(true)));
    assert_eq!(
        resolved.get("city"),
        Some(&SettingValue::Text("Oslo".into()))
    );
    assert_eq!(
        issues,
        [
            FieldIssue::WrongKind { key: "on".into() },
            FieldIssue::WrongKind { key: "city".into() }
        ]
    );
}

#[test]
fn a_removed_option_falls_back_to_the_default_with_an_issue() {
    let schema = sample();
    let stored = values(&[
        ("mode", SettingValue::Choice("turbo".into())),
        ("tags", chosen(&["a", "z"])),
    ]);
    let (resolved, issues) = resolve(&schema, &stored);
    assert_eq!(
        resolved.get("mode"),
        Some(&SettingValue::Choice("fast".into()))
    );
    assert_eq!(resolved.get("tags"), Some(&chosen(&["a"])));
    assert_eq!(
        issues,
        [
            FieldIssue::RemovedOption {
                key: "mode".into(),
                option: "turbo".into()
            },
            FieldIssue::RemovedOption {
                key: "tags".into(),
                option: "z".into()
            }
        ]
    );
}

#[test]
fn out_of_bounds_values_fall_back_to_the_default_with_an_issue() {
    let schema = sample();
    let stored = values(&[
        ("count", SettingValue::Number(99)),
        ("city", SettingValue::Text("a very long city name".into())),
        ("tags", chosen(&["a", "b", "c"])),
    ]);
    let (resolved, issues) = resolve(&schema, &stored);
    assert_eq!(resolved.get("count"), Some(&SettingValue::Number(4)));
    assert_eq!(
        resolved.get("city"),
        Some(&SettingValue::Text("Oslo".into()))
    );
    assert_eq!(resolved.get("tags"), Some(&chosen(&["a"])));
    assert_eq!(issues.len(), 3);
    assert!(
        issues
            .iter()
            .all(|issue| matches!(issue, FieldIssue::Invalid { .. }))
    );
}

#[test]
fn stored_text_is_sanitized_like_an_edit() {
    let schema = sample();
    let stored = values(&[("city", SettingValue::Text("Ro\u{202e}me\n".into()))]);
    let (resolved, issues) = resolve(&schema, &stored);
    assert_eq!(
        resolved.get("city"),
        Some(&SettingValue::Text("Rome".into()))
    );
    assert!(issues.is_empty());
}

fn renamed() -> SettingsSchema {
    let mut field = field("town", text("Oslo", 8));
    field.renamed_from = Some(Cow::Borrowed("city"));
    let mut schema = schema_of(vec![field]);
    schema.version = 2;
    schema
}

#[test]
fn a_renamed_field_takes_the_value_stored_under_its_old_key() {
    let (resolved, issues) = resolve(
        &renamed(),
        &values(&[("city", SettingValue::Text("Rome".into()))]),
    );
    assert_eq!(
        resolved.get("town"),
        Some(&SettingValue::Text("Rome".into()))
    );
    assert!(resolved.get("city").is_none());
    assert!(issues.is_empty());
}

#[test]
fn a_value_under_the_new_key_wins_over_the_old_key() {
    let (resolved, _) = resolve(
        &renamed(),
        &values(&[
            ("city", SettingValue::Text("Rome".into())),
            ("town", SettingValue::Text("Pisa".into())),
        ]),
    );
    assert_eq!(
        resolved.get("town"),
        Some(&SettingValue::Text("Pisa".into()))
    );
}

#[test]
fn a_renamed_value_of_the_wrong_kind_is_reported_under_the_new_key() {
    let (resolved, issues) = resolve(&renamed(), &values(&[("city", SettingValue::Bool(true))]));
    assert_eq!(
        resolved.get("town"),
        Some(&SettingValue::Text("Oslo".into()))
    );
    assert_eq!(issues, [FieldIssue::WrongKind { key: "town".into() }]);
}

#[test]
fn renamed_from_must_be_a_fresh_valid_key() {
    let check = |old: &'static str| {
        let mut renamed = field("town", text("", 8));
        renamed.renamed_from = Some(Cow::Borrowed(old));
        schema_of(vec![renamed, field("city", text("", 8))]).validate(&[])
    };
    assert!(matches!(
        check("city"),
        Err(SchemaError::RenamedFrom { .. })
    ));
    assert!(matches!(
        check("Bad Key"),
        Err(SchemaError::RenamedFrom { .. })
    ));
    assert!(matches!(
        check("town"),
        Err(SchemaError::RenamedFrom { .. })
    ));
    let mut moved = field("town", text("", 8));
    moved.renamed_from = Some(Cow::Borrowed("village"));
    let mut twin = field("hamlet", text("", 8));
    twin.renamed_from = Some(Cow::Borrowed("village"));
    assert!(matches!(
        schema_of(vec![moved.clone(), twin]).validate(&[]),
        Err(SchemaError::RenamedFrom { .. })
    ));
    assert!(schema_of(vec![moved]).validate(&[]).is_ok());
}
