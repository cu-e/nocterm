use std::borrow::Cow;

use super::{fixtures::*, *};
use crate::{CatalogError, PluginCatalog, PluginId, SystemPlugin};

fn own() -> Vec<ContributionId> {
    vec![ContributionId::new("demo.widget")]
}

fn invalid(schema: SettingsSchema) -> SchemaError {
    schema.validate(&own()).unwrap_err()
}

#[test]
fn a_schema_of_every_kind_is_valid() {
    assert_eq!(sample().validate(&own()), Ok(()));
    assert_eq!(SettingsSchema::NONE.validate(&[]), Ok(()));
}

#[test]
fn more_than_eight_groups_are_refused() {
    let groups = (0..9)
        .map(|_| group("G", None, Vec::new()))
        .collect::<Vec<_>>();
    let schema = SettingsSchema {
        groups: groups.into(),
    };
    assert_eq!(
        invalid(schema),
        SchemaError::TooMany {
            limit: SchemaLimit::Groups,
            max: 8
        }
    );
}

#[test]
fn more_than_thirty_two_fields_are_refused() {
    let keys: Vec<&'static str> = (0..33)
        .map(|n| &*Box::leak(format!("k{n}").into_boxed_str()))
        .collect();
    let fields = keys
        .iter()
        .map(|key| field(key, FieldKind::Toggle { default: false }))
        .collect();
    assert_eq!(
        invalid(schema_of(fields)),
        SchemaError::TooMany {
            limit: SchemaLimit::Fields,
            max: 32
        }
    );
}

#[test]
fn more_than_sixteen_options_are_refused() {
    let keys: Vec<&'static str> = (0..17)
        .map(|n| &*Box::leak(format!("o{n}").into_boxed_str()))
        .collect();
    let kind = multi(&keys, &[], false, 1);
    assert_eq!(
        invalid(schema_of(vec![field("many", kind)])),
        SchemaError::TooMany {
            limit: SchemaLimit::Options,
            max: 16
        }
    );
}

#[test]
fn keys_follow_the_key_syntax_and_are_unique() {
    for bad in [
        "",
        "Upper",
        "1st",
        "has-dash",
        "a_very_long_key_that_goes_past_thirty_two",
    ] {
        let schema = schema_of(vec![field(bad, FieldKind::Toggle { default: false })]);
        assert!(
            matches!(invalid(schema), SchemaError::Key { .. }),
            "{bad:?}"
        );
    }
    let twice = schema_of(vec![
        field("same", FieldKind::Toggle { default: false }),
        field("same", FieldKind::Toggle { default: true }),
    ]);
    assert!(matches!(invalid(twice), SchemaError::DuplicateKey { key } if key == "same"));
}

#[test]
fn duplicate_keys_are_found_across_groups() {
    let schema = SettingsSchema {
        groups: vec![
            group(
                "A",
                None,
                vec![field("same", FieldKind::Toggle { default: false })],
            ),
            group(
                "B",
                None,
                vec![field("same", FieldKind::Toggle { default: false })],
            ),
        ]
        .into(),
    };
    assert!(matches!(invalid(schema), SchemaError::DuplicateKey { .. }));
}

#[test]
fn labels_and_help_are_short_plain_text() {
    let mut long = field("a", FieldKind::Toggle { default: false });
    long.label = Cow::Owned("x".repeat(49));
    assert!(matches!(
        invalid(schema_of(vec![long])),
        SchemaError::Label { .. }
    ));
    let mut helped = field("a", FieldKind::Toggle { default: false });
    helped.help = Some(Cow::Owned("x".repeat(161)));
    assert!(matches!(
        invalid(schema_of(vec![helped])),
        SchemaError::Label { .. }
    ));
    let mut controlled = field("a", FieldKind::Toggle { default: false });
    controlled.label = Cow::Borrowed("two\nlines");
    assert!(matches!(
        invalid(schema_of(vec![controlled])),
        SchemaError::Label { .. }
    ));
    let mut spoofed = field("a", FieldKind::Toggle { default: false });
    spoofed.label = Cow::Borrowed("evil\u{202E}txt");
    assert!(matches!(
        invalid(schema_of(vec![spoofed])),
        SchemaError::Label { .. }
    ));
    let mut empty = field("a", FieldKind::Toggle { default: false });
    empty.label = Cow::Borrowed("");
    assert!(matches!(
        invalid(schema_of(vec![empty])),
        SchemaError::Label { .. }
    ));
}

#[test]
fn a_group_title_is_plain_text() {
    let schema = SettingsSchema {
        groups: vec![group("bad\ttitle", None, Vec::new())].into(),
    };
    assert_eq!(invalid(schema), SchemaError::GroupTitle { index: 0 });
}

#[test]
fn options_are_present_well_formed_and_unique() {
    let empty = FieldKind::Choice {
        options: options(&[]),
        default: Cow::Borrowed("x"),
    };
    assert!(matches!(
        invalid(schema_of(vec![field("c", empty)])),
        SchemaError::Options { .. }
    ));
    let repeated = FieldKind::Choice {
        options: options(&["a", "a"]),
        default: Cow::Borrowed("a"),
    };
    assert!(matches!(
        invalid(schema_of(vec![field("c", repeated)])),
        SchemaError::Options { .. }
    ));
    let malformed = FieldKind::Choice {
        options: options(&["Bad Key"]),
        default: Cow::Borrowed("Bad Key"),
    };
    assert!(matches!(
        invalid(schema_of(vec![field("c", malformed)])),
        SchemaError::Options { .. }
    ));
}

#[test]
fn defaults_must_be_valid_for_their_kind() {
    let unknown_choice = FieldKind::Choice {
        options: options(&["a"]),
        default: Cow::Borrowed("b"),
    };
    let cases = [
        unknown_choice,
        multi(&["a", "b"], &["c"], false, 2),
        multi(&["a", "b"], &["a", "a"], false, 2),
        multi(&["a", "b"], &["a", "b"], false, 1),
        number(0, 10, 5, 3),
        number(0, 10, 1, 11),
        text("a\nb", 8),
        text("too long for it", 4),
    ];
    for kind in cases {
        let error = invalid(schema_of(vec![field("f", kind.clone())]));
        assert!(
            matches!(error, SchemaError::Default { .. }),
            "{kind:?}: {error:?}"
        );
    }
    let placeholder = FieldKind::Text {
        default: Cow::Borrowed(""),
        max_len: 8,
        placeholder: Some(Cow::Borrowed("a\u{200B}b")),
    };
    assert!(matches!(
        invalid(schema_of(vec![field("f", placeholder)])),
        SchemaError::Default { .. }
    ));
}

#[test]
fn ranges_steps_maxima_and_text_lengths_are_bounded() {
    let cases = [
        number(5, 1, 1, 3),
        number(0, 10, 0, 0),
        number(0, 10, -1, 0),
        multi(&["a"], &[], false, 0),
        multi(&["a"], &[], false, 2),
        text("", 0),
        text("", 257),
    ];
    for kind in cases {
        let error = invalid(schema_of(vec![field("f", kind.clone())]));
        assert!(
            matches!(error, SchemaError::Range { .. }),
            "{kind:?}: {error:?}"
        );
    }
    assert_eq!(
        schema_of(vec![field("f", text("", 256))]).validate(&own()),
        Ok(())
    );
}

#[test]
fn a_group_may_name_only_a_contribution_of_the_plugin() {
    let schema = SettingsSchema {
        groups: vec![group("G", Some("demo.widget"), Vec::new())].into(),
    };
    assert_eq!(schema.validate(&own()), Ok(()));
    let foreign = SettingsSchema {
        groups: vec![group("G", Some("other.widget"), Vec::new())].into(),
    };
    assert_eq!(
        invalid(foreign),
        SchemaError::UnknownContribution {
            id: ContributionId::new("other.widget")
        }
    );
}

#[test]
fn defaults_hold_every_field_default() {
    let values = sample().defaults();
    assert_eq!(values.get("on"), Some(&SettingValue::Bool(true)));
    assert_eq!(
        values.get("mode"),
        Some(&SettingValue::Choice("fast".into()))
    );
    assert_eq!(
        values.get("tags"),
        Some(&SettingValue::Choices(vec!["a".into()]))
    );
    assert_eq!(values.get("count"), Some(&SettingValue::Number(4)));
    assert_eq!(values.get("city"), Some(&SettingValue::Text("Oslo".into())));
    assert_eq!(values.iter().count(), 5);
}

#[test]
fn a_contribution_sees_its_groups_and_not_the_plugin_level_ones() {
    let schema = SettingsSchema {
        groups: vec![
            group(
                "Plugin",
                None,
                vec![field("a", FieldKind::Toggle { default: false })],
            ),
            group(
                "Widget",
                Some("demo.widget"),
                vec![field("b", FieldKind::Toggle { default: false })],
            ),
            group(
                "Other",
                Some("demo.panel"),
                vec![field("c", FieldKind::Toggle { default: false })],
            ),
        ]
        .into(),
    };
    let id = ContributionId::new("demo.widget");
    let titles: Vec<_> = schema
        .for_contribution(&id)
        .map(|g| g.title.to_string())
        .collect();
    assert_eq!(titles, ["Widget"]);
    assert_eq!(schema.field("c").map(|f| f.key.as_ref()), Some("c"));
    assert!(schema.field("missing").is_none());
}

fn catalog_of(plugin: &'static SystemPlugin) -> Result<PluginCatalog, CatalogError> {
    use crate::{GroupId, GroupSpec};
    let groups = [GroupSpec {
        id: GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    }];
    PluginCatalog::new(&groups, &[], &[plugin])
}

fn demo(schema: SettingsSchema, section: Option<&'static str>) -> &'static SystemPlugin {
    Box::leak(Box::new(SystemPlugin {
        id: PluginId::new("demo"),
        name: "Demo",
        description: "A test plugin",
        required: false,
        contributions: &[],
        settings_section: section,
        settings: schema,
    }))
}

#[test]
fn the_catalog_refuses_an_invalid_schema_and_keeps_its_cause() {
    let bad = schema_of(vec![field("Bad", FieldKind::Toggle { default: false })]);
    let error = catalog_of(demo(bad, Some("demo"))).unwrap_err();
    let CatalogError::Schema { plugin, source } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(plugin.as_str(), "demo");
    assert!(matches!(source, SchemaError::Key { .. }));
    let cause = std::error::Error::source(&error).expect("the schema error is the source");
    assert_eq!(cause.to_string(), source.to_string());
}

#[test]
fn the_catalog_needs_a_settings_section_for_declared_fields() {
    let schema = schema_of(vec![field("on", FieldKind::Toggle { default: false })]);
    let error = catalog_of(demo(schema.clone(), None)).unwrap_err();
    assert!(matches!(error, CatalogError::SettingsSection { plugin } if plugin.as_str() == "demo"));
    assert!(catalog_of(demo(schema, Some("demo"))).is_ok());
    assert!(catalog_of(demo(SettingsSchema::NONE, None)).is_ok());
}
