use alloc::{collections::BTreeSet, string::String, vec::Vec};

use serde::Serialize;

use super::support::host;
use crate::{
    ChoiceOptionDto, ContributionDto, EFFECT_REQUESTS, EVENTS, Effect, Event, FILTERS, FieldDto,
    FieldKindName, GroupDto, HOST_ERRORS, HostError, Key, KeyType, Kind, LIMITS, LocalId,
    MANIFEST_KEYS, Manifest, ManifestDto, NodeKind, PluginIcon, REFRESHES, RUNTIMES, Refresh,
    Runtime, SETTINGS_KINDS, SettingsDto, Tone, Zone,
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

fn table(name: &str) -> &'static [Key] {
    MANIFEST_KEYS
        .iter()
        .find(|(table, _)| *table == name)
        .unwrap_or_else(|| panic!("{name} is not listed"))
        .1
}

fn listed(name: &str) -> BTreeSet<String> {
    table(name)
        .iter()
        .map(|key| String::from(key.name))
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
    assert_eq!(MANIFEST_KEYS.len(), 8);
}

/// A manifest with one instance of every table and none of the optional keys but the ones the
/// tables below need.
fn base() -> serde_json::Value {
    serde_json::json!({
        "api": "0.1",
        "publisher": "a",
        "id": "b",
        "name": "N",
        "contribution": [{
            "local": "face",
            "kind": "widget",
            "title": "T",
            "view": { "nodes": [{ "kind": "text", "text": "x" }] }
        }],
        "settings": {
            "version": 1,
            "group": [{
                "title": "G",
                "field": [{
                    "key": "k",
                    "label": "L",
                    "kind": "choice",
                    "default": "o",
                    "options": [{ "key": "o", "label": "O" }]
                }]
            }]
        }
    })
}

/// The instance of a table in `doc`.
fn instance<'a>(doc: &'a mut serde_json::Value, table: &str) -> &'a mut serde_json::Value {
    let path: &[&str] = match table {
        "manifest" => &[],
        "contribution" => &["contribution", "0"],
        "contribution.view" => &["contribution", "0", "view"],
        "contribution.view.nodes" => &["contribution", "0", "view", "nodes", "0"],
        "settings" => &["settings"],
        "settings.group" => &["settings", "group", "0"],
        "settings.group.field" => &["settings", "group", "0", "field", "0"],
        "settings.group.field.options" => &["settings", "group", "0", "field", "0", "options", "0"],
        other => panic!("{other} has no instance"),
    };
    path.iter()
        .fold(doc, |at, step| match step.parse::<usize>() {
            Ok(index) => &mut at[index],
            Err(_) => &mut at[*step],
        })
}

/// Whether the keys and their forms are ones the manifest reads.
fn readable(doc: &serde_json::Value) -> bool {
    serde_json::from_value::<ManifestDto>(doc.clone()).is_ok()
}

/// Whether the host takes the package: it reads, and it passes the checks made before consent.
fn accepted(doc: &serde_json::Value) -> bool {
    serde_json::from_value::<ManifestDto>(doc.clone())
        .ok()
        .is_some_and(|dto| Manifest::validate(&dto, &host()).is_ok())
}

fn with(table: &str, key: &str, value: Option<serde_json::Value>) -> serde_json::Value {
    let mut doc = base();
    let at = instance(&mut doc, table).as_object_mut().unwrap();
    match value {
        Some(value) => at.insert(key.into(), value),
        None => at.remove(key),
    };
    doc
}

/// The names of a vocabulary, as the surface file lists them.
fn vocabulary(name: &str) -> Vec<&'static str> {
    match name {
        "runtimes" => RUNTIMES.to_vec(),
        "kinds" => Kind::ALL.map(Kind::name).to_vec(),
        "zones" => Zone::ALL.map(Zone::name).to_vec(),
        "icons" => PluginIcon::ALL.map(PluginIcon::name).to_vec(),
        "node_kinds" => NodeKind::NAMES.to_vec(),
        "tones" => Tone::ALL.map(Tone::name).to_vec(),
        "settings_kinds" => SETTINGS_KINDS.to_vec(),
        other => panic!("{other} is not a vocabulary of the surface"),
    }
}

#[test]
fn a_package_without_a_required_key_is_refused_and_one_without_an_optional_key_still_reads() {
    let dto = serde_json::from_value::<ManifestDto>(base()).unwrap();
    Manifest::validate(&dto, &host()).expect("the base package is valid");
    for (name, keys) in MANIFEST_KEYS {
        for key in *keys {
            let without = with(name, key.name, None);
            if key.required {
                assert!(!accepted(&without), "{name}.{} is required", key.name);
            } else {
                assert!(readable(&without), "{name}.{} is optional", key.name);
            }
        }
    }
}

/// A value of the declared type, for the keys whose type is one text, number or list.
fn sample(ty: KeyType) -> Option<serde_json::Value> {
    use serde_json::json;
    match ty {
        KeyType::Text => Some(json!("x")),
        KeyType::Integer { .. } => Some(json!(7)),
        KeyType::Bool => Some(json!(true)),
        KeyType::TextList => Some(json!(["x"])),
        KeyType::TextMap => Some(json!({ "x": "y" })),
        KeyType::Choice(name) => Some(json!(vocabulary(name)[0])),
        KeyType::Table | KeyType::TableList | KeyType::Any => None,
    }
}

#[test]
fn a_key_takes_a_value_of_its_declared_type_and_refuses_the_other_forms() {
    use serde_json::json;
    let forms = [json!("zzz"), json!(7), json!(true), json!(["zzz"])];
    for (name, keys) in MANIFEST_KEYS {
        for key in *keys {
            if let Some(value) = sample(key.ty) {
                assert!(
                    readable(&with(name, key.name, Some(value.clone()))),
                    "{name}.{} should take {value}",
                    key.name
                );
            }
            if key.ty == KeyType::Any {
                continue;
            }
            for form in &forms {
                let right = sample(key.ty).as_ref() == Some(form);
                let fits = right
                    || (matches!(key.ty, KeyType::Text) && form.is_string())
                    || (matches!(key.ty, KeyType::TextList) && form == &json!(["zzz"]));
                assert_eq!(
                    readable(&with(name, key.name, Some(form.clone()))),
                    fits,
                    "{name}.{} ({}) with {form}",
                    key.name,
                    key.ty.name()
                );
            }
        }
    }
}

#[test]
fn an_integer_key_reads_the_whole_listed_range_and_nothing_outside_it() {
    use serde_json::json;
    let mut checked = 0;
    for (name, keys) in MANIFEST_KEYS {
        for key in *keys {
            let KeyType::Integer { min, max } = key.ty else {
                continue;
            };
            let reads = |number: i64| readable(&with(name, key.name, Some(json!(number))));
            assert!(reads(max), "{name}.{} takes {max}", key.name);
            assert!(!reads(max + 1), "{name}.{} refuses {}", key.name, max + 1);
            // The host refuses a step below `min` after reading it; see the next test.
            if key.name != "step" {
                assert!(reads(min), "{name}.{} takes {min}", key.name);
                assert!(!reads(min - 1), "{name}.{} refuses {}", key.name, min - 1);
            }
            checked += 1;
        }
    }
    assert_eq!(
        checked, 8,
        "order, parent, version, max_chosen, max_len, min, max, step"
    );
}

/// A package with one number setting.
fn number_setting(min: i64, step: i64) -> serde_json::Value {
    let mut doc = base();
    *instance(&mut doc, "settings.group.field") = serde_json::json!({
        "key": "k", "label": "L", "kind": "number", "min": min, "max": 10, "step": step
    });
    doc
}

#[test]
fn a_number_setting_may_start_below_zero_and_the_listed_step_range_is_what_the_host_takes() {
    let KeyType::Integer { min: lowest, .. } = table("settings.group.field")
        .iter()
        .find(|key| key.name == "step")
        .unwrap()
        .ty
    else {
        panic!("step is a number");
    };
    assert!(
        accepted(&number_setting(-12, lowest)),
        "a UTC offset starts at -12"
    );
    assert!(
        !accepted(&number_setting(0, lowest - 1)),
        "a step below the range"
    );
}

/// The request of `id` as the surface lists it.
fn request(id: &str) -> &'static [Key] {
    EFFECT_REQUESTS
        .iter()
        .find(|(name, _)| *name == id)
        .unwrap_or_else(|| panic!("{id} is not listed"))
        .1
}

/// Checks the listed fields of the request `id` against the type `T` that carries it: the names
/// are the ones `sample` has, a required field cannot be left out and an optional one can, and
/// each field holds the kind of value it is listed with.
fn assert_request_is_what_the_type_takes<T>(id: &str, sample: &T)
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let full = serde_json::to_value(sample).unwrap();
    let object = full.as_object().unwrap();
    let listed: BTreeSet<&str> = request(id).iter().map(|key| key.name).collect();
    assert_eq!(
        object.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        listed,
        "{id}"
    );
    for key in request(id) {
        let mut without = full.clone();
        without.as_object_mut().unwrap().remove(key.name);
        assert_eq!(
            serde_json::from_value::<T>(without).is_ok(),
            !key.required,
            "{id}.{} is {} but the type disagrees",
            key.name,
            if key.required { "required" } else { "optional" },
        );
        let value = &object[key.name];
        match key.ty {
            KeyType::Text => assert!(value.is_string(), "{id}.{}", key.name),
            KeyType::Bool => assert!(value.is_boolean(), "{id}.{}", key.name),
            KeyType::Integer { min, max } => {
                let number = value
                    .as_i64()
                    .unwrap_or_else(|| panic!("{id}.{}", key.name));
                assert!((min..=max).contains(&number), "{id}.{}", key.name);
            }
            KeyType::Choice(vocabulary) => {
                assert_eq!(vocabulary, "notice_tones", "{id}.{}", key.name);
                let tone = value.as_str().unwrap();
                assert!(NoticeTone::ALL.iter().any(|t| t.name() == tone));
            }
            other => panic!("{id}.{}: {other:?} is no request field", key.name),
        }
    }
}

#[test]
fn the_listed_effect_requests_are_what_the_effects_take_and_carry() {
    let local = LocalId::parse("a").unwrap();
    assert_request_is_what_the_type_takes(
        "notifications.post",
        &Post {
            tone: NoticeTone::Info,
            text: String::new(),
            notice_key: Some(String::new()),
        },
    );
    assert_request_is_what_the_type_takes(
        "panels.set-open",
        &SetOpen {
            local: local.clone(),
            open: true,
        },
    );
    assert_request_is_what_the_type_takes(
        "panels.set-badge",
        &SetBadge {
            local,
            badge: Some(3),
        },
    );
    assert_request_is_what_the_type_takes(
        "storage.set",
        &Set {
            key: String::new(),
            value: String::new(),
        },
    );
    assert_request_is_what_the_type_takes("storage.remove", &Remove { key: String::new() });
    assert_eq!(EFFECT_REQUESTS.len(), 5, "a new effect needs a case here");
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
    let runtimes: Vec<Runtime> = RUNTIMES
        .iter()
        .map(|name| serde_json::from_value(serde_json::Value::from(*name)).unwrap())
        .collect();
    assert_eq!(runtimes, [Runtime::Declarative, Runtime::Wasm]);
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
