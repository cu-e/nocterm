use alloc::{collections::BTreeSet, string::String, vec::Vec};

use serde::Serialize;

use crate::{
    ChoiceOptionDto, ContributionDto, EFFECT_REQUESTS, EVENTS, Effect, Event, FILTERS, FieldDto,
    FieldKindName, GroupDto, HOST_ERRORS, HostError, LIMITS, LocalId, MANIFEST_TABLES, ManifestDto,
    REFRESHES, RUNTIMES, Refresh, Runtime, SETTINGS_KINDS, SettingsDto,
    capability::{
        Fields,
        clock::ClockNow,
        monitor::Usage,
        notifications::{NoticeTone, Post},
        panels::{SetBadge, SetOpen},
        storage::{Remove, Set},
        transfers::Counts,
        workspace::{WorkspaceKind, WorkspaceState},
    },
    template::{NodeDto, ViewDto},
};

fn keys(value: &impl Serialize) -> BTreeSet<String> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

fn listed(table: &str) -> BTreeSet<String> {
    MANIFEST_TABLES
        .iter()
        .find(|(name, _)| *name == table)
        .unwrap_or_else(|| panic!("{table} is not listed"))
        .1
        .iter()
        .map(|key| String::from(*key))
        .collect()
}

#[test]
fn the_listed_manifest_keys_are_exactly_what_the_dtos_accept() {
    assert_eq!(listed("manifest"), keys(&ManifestDto::default()));
    assert_eq!(listed("contribution"), keys(&ContributionDto::default()));
    assert_eq!(listed("contribution.view"), keys(&ViewDto::default()));
    assert_eq!(listed("contribution.view.nodes"), keys(&NodeDto::default()));
    assert_eq!(listed("settings"), keys(&SettingsDto::default()));
    assert_eq!(listed("settings.group"), keys(&GroupDto::default()));
    assert_eq!(listed("settings.group.field"), keys(&FieldDto::default()));
    assert_eq!(
        listed("settings.group.field.options"),
        keys(&ChoiceOptionDto::default())
    );
    assert_eq!(MANIFEST_TABLES.len(), 8);
}

#[test]
fn the_listed_effect_requests_are_exactly_what_the_effects_carry() {
    let local = LocalId::parse("a").unwrap();
    let requests = [
        (
            "notifications.post",
            keys(&Post {
                tone: NoticeTone::Info,
                text: String::new(),
                notice_key: None,
            }),
        ),
        (
            "panels.set-open",
            keys(&SetOpen {
                local: local.clone(),
                open: true,
            }),
        ),
        ("panels.set-badge", keys(&SetBadge { local, badge: None })),
        (
            "storage.set",
            keys(&Set {
                key: String::new(),
                value: String::new(),
            }),
        ),
        ("storage.remove", keys(&Remove { key: String::new() })),
    ];
    assert_eq!(requests.len(), EFFECT_REQUESTS.len());
    for (id, found) in requests {
        let listed: BTreeSet<String> = EFFECT_REQUESTS
            .iter()
            .find(|(name, _)| *name == id)
            .unwrap()
            .1
            .iter()
            .map(|key| String::from(*key))
            .collect();
        assert_eq!(listed, found, "{id}");
    }
}

fn kind_name(kind: FieldKindName) -> &'static str {
    match kind {
        FieldKindName::Toggle => "toggle",
        FieldKindName::Choice => "choice",
        FieldKindName::MultiChoice => "multi-choice",
        FieldKindName::Number => "number",
        FieldKindName::Text => "text",
    }
}

#[test]
fn listed_names_are_the_names_the_types_use() {
    for name in SETTINGS_KINDS {
        let kind: FieldKindName = serde_json::from_value(name.into()).unwrap();
        assert_eq!(kind_name(kind), name);
    }
    for name in RUNTIMES {
        assert_eq!(
            serde_json::from_value::<Runtime>(name.into()).unwrap(),
            Runtime::Declarative
        );
    }
    let events = [
        Event::Activated { key: String::new() },
        Event::Toggled {
            key: String::new(),
            on: true,
        },
        Event::VisibilityChanged { visible: true },
        Event::SettingsChanged,
    ];
    let named: Vec<String> = events
        .iter()
        .map(|event| match serde_json::to_value(event).unwrap() {
            serde_json::Value::String(name) => name,
            other => other.as_object().unwrap().keys().next().unwrap().clone(),
        })
        .collect();
    assert_eq!(named, EVENTS);
    let errors = [
        HostError::NotGranted,
        HostError::Denied,
        HostError::RateLimited { retry_after_ms: 0 },
        HostError::Unavailable,
    ];
    let named: Vec<String> = errors
        .iter()
        .map(|error| match serde_json::to_value(error).unwrap() {
            serde_json::Value::String(name) => name,
            other => other.as_object().unwrap().keys().next().unwrap().clone(),
        })
        .collect();
    assert_eq!(named, HOST_ERRORS);
    let refreshes = [
        Refresh::NextSecond,
        Refresh::NextMinute,
        Refresh::AfterMs(250),
    ];
    let named: Vec<String> = refreshes
        .iter()
        .map(|refresh| match serde_json::to_value(refresh).unwrap() {
            serde_json::Value::String(name) => name,
            other => other.as_object().unwrap().keys().next().unwrap().clone(),
        })
        .collect();
    assert_eq!(named, REFRESHES);
}

#[test]
fn limits_and_filters_are_listed_once_and_sorted() {
    let names: Vec<&str> = LIMITS.iter().map(|(name, _)| *name).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted);
    assert!(LIMITS.iter().all(|(_, value)| *value > 0));
    assert_eq!(FILTERS.len(), 4);
    let _ = Effect::StorageRemove(Remove { key: String::new() });
}

fn field_names<T: Fields>(value: &T) -> BTreeSet<String> {
    let names: BTreeSet<String> = T::FIELDS
        .iter()
        .map(|(name, _)| String::from(*name))
        .collect();
    for name in &names {
        assert!(value.field(name).is_some(), "{name} is listed but unread");
    }
    names
}

#[test]
fn a_template_field_is_named_as_the_data_serializes_it() {
    let clock = ClockNow {
        unix_minutes: 1,
        utc_offset_seconds: 0,
    };
    let usage = Usage {
        cpu_percent: 1.0,
        memory_percent: 2.0,
    };
    let workspace = WorkspaceState {
        kind: WorkspaceKind::Local,
        connected: true,
    };
    let counts = Counts {
        active: 1,
        failed: 0,
    };
    assert_eq!(field_names(&clock), keys(&clock));
    assert_eq!(field_names(&usage), keys(&usage));
    assert_eq!(field_names(&workspace), keys(&workspace));
    assert_eq!(field_names(&counts), keys(&counts));
    assert!(field_names(&clock).contains("unix_minutes"));
}
