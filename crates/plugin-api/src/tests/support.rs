//! Shared fixtures.

use alloc::{borrow::Cow, string::ToString, vec, vec::Vec};

use nocterm_plugin_model::{FieldGroup, FieldKind, FieldSpec, SettingsSchema};

use crate::{
    ContributionContext, HostContext, Kind, Manifest, ManifestDto, Observed, Snapshot, Zone,
    capability::clock::ClockNow,
};

pub(super) const EXAMPLE: &str = r#"
api = "0.1"
publisher = "acme"
id = "clock"
name = "Acme Clock"
description = "Shows the time"
capabilities = ["clock.now"]
optional_capabilities = ["monitor.usage"]

[reasons]
"clock.now" = "To show the time"

[[contribution]]
local = "face"
kind = "widget"
title = "Clock"
icon = "puzzle"

[contribution.view]
nodes = [
  { kind = "row" },
  { kind = "icon", parent = 0, icon = "puzzle" },
  { kind = "text", parent = 0, text = "{clock.now|hh-mm} {settings.city}" },
]

[settings]
version = 2

[[settings.group]]
title = "General"

[[settings.group.field]]
key = "city"
label = "City"
kind = "text"
max_len = 40
"#;

pub(super) fn dto(text: &str) -> ManifestDto {
    toml::from_str(text).expect("the fixture is valid TOML for the DTO")
}

pub(super) fn example() -> ManifestDto {
    dto(EXAMPLE)
}

pub(super) fn host() -> HostContext<'static> {
    HostContext::new(crate::API_VERSION, &["Files", "Status bar"])
}

pub(super) fn manifest() -> Manifest {
    Manifest::validate(&example(), &host()).expect("the example is valid")
}

pub(super) fn snapshot() -> Snapshot {
    Snapshot::new(ContributionContext {
        kind: Kind::Widget,
        zone: Zone::Right,
    })
}

/// A snapshot where it is 13:05 UTC at offset +01:00.
pub(super) fn with_clock(mut snapshot: Snapshot) -> Snapshot {
    snapshot.observations.insert(
        "clock.now".to_string(),
        Ok(Observed::Clock(ClockNow {
            unix_minutes: 13 * 60 + 5,
            utc_offset_seconds: 3600,
        })),
    );
    snapshot
}

pub(super) fn text_schema(key: &str) -> SettingsSchema {
    let field = FieldSpec {
        key: Cow::Owned(key.to_string()),
        label: Cow::Borrowed("Label"),
        help: None,
        kind: FieldKind::Text {
            default: Cow::Borrowed(""),
            max_len: 40,
            placeholder: None,
        },
        renamed_from: None,
    };
    SettingsSchema {
        version: 1,
        groups: Cow::Owned(vec![FieldGroup {
            title: Cow::Borrowed("General"),
            contribution: None,
            fields: Cow::Owned(Vec::from([field])),
        }]),
    }
}
