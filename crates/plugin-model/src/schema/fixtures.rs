//! Small builders for schemas in tests.

use std::borrow::Cow;

use super::{ChoiceOption, FieldGroup, FieldKind, FieldSpec, SettingsSchema};
use crate::ContributionId;

pub(super) fn options(keys: &[&'static str]) -> Cow<'static, [ChoiceOption]> {
    keys.iter()
        .map(|key| ChoiceOption {
            key: Cow::Borrowed(*key),
            label: Cow::Borrowed(*key),
            help: None,
        })
        .collect::<Vec<_>>()
        .into()
}

pub(super) fn field(key: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec {
        key: Cow::Borrowed(key),
        label: Cow::Borrowed("Label"),
        help: None,
        kind,
    }
}

pub(super) fn multi(
    keys: &[&'static str],
    default: &[&'static str],
    ordered: bool,
    max: u8,
) -> FieldKind {
    FieldKind::MultiChoice {
        options: options(keys),
        default: default
            .iter()
            .map(|key| Cow::Borrowed(*key))
            .collect::<Vec<_>>()
            .into(),
        ordered,
        max,
    }
}

pub(super) fn number(min: i32, max: i32, step: i32, default: i32) -> FieldKind {
    FieldKind::Number {
        min,
        max,
        step,
        default,
        unit: None,
    }
}

pub(super) fn text(default: &'static str, max_len: u16) -> FieldKind {
    FieldKind::Text {
        default: Cow::Borrowed(default),
        max_len,
        placeholder: None,
    }
}

pub(super) fn group(
    title: &'static str,
    contribution: Option<&'static str>,
    fields: Vec<FieldSpec>,
) -> FieldGroup {
    FieldGroup {
        title: Cow::Borrowed(title),
        contribution: contribution.map(ContributionId::new),
        fields: fields.into(),
    }
}

pub(super) fn schema_of(fields: Vec<FieldSpec>) -> SettingsSchema {
    SettingsSchema {
        groups: vec![group("Group", None, fields)].into(),
    }
}

/// One field of every kind: `on`, `mode`, `tags` (ordered, up to two), `count` (2 to 10 in
/// steps of 2) and `city` (up to 8 characters).
pub(super) fn sample() -> SettingsSchema {
    schema_of(vec![
        field("on", FieldKind::Toggle { default: true }),
        field(
            "mode",
            FieldKind::Choice {
                options: options(&["fast", "slow"]),
                default: Cow::Borrowed("fast"),
            },
        ),
        field("tags", multi(&["a", "b", "c"], &["a"], true, 2)),
        field("count", number(2, 10, 2, 4)),
        field("city", text("Oslo", 8)),
    ])
}
