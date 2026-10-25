//! The plugin API surface file, `crates/plugin-api/api-surface.json`.
//!
//! The file lists every name and limit of the user plugin contract: capabilities, node kinds,
//! tones, icons, zones, manifest keys, settings kinds, filters, events, errors and limits. It is
//! written from the api crate's constants, so it cannot disagree with them, and `--check` keeps
//! it fresh. A release comparison (from X-ship) fails on any removal, rename, type change or
//! tightened limit that is not a deprecation.

use anyhow::Context as _;
use nocterm_plugin_api::{
    API_VERSION, CAPABILITIES, CapabilityEntry, EFFECT_REQUESTS, EVENTS, EffectTarget, FILTERS,
    Family, HOST_ERRORS, Kind, LIMITS, MANIFEST_TABLES, NodeKind, PluginIcon, REFRESHES, RUNTIMES,
    SETTINGS_KINDS, Tone, Zone,
    capability::{fields_of, notifications::NoticeTone, workspace::WorkspaceKind},
};
use serde_json::{Map, Value, json};

/// The path of the surface file, relative to the repository root.
pub(crate) const PATH: &str = "crates/plugin-api/api-surface.json";

pub(crate) fn render() -> anyhow::Result<String> {
    let mut text =
        serde_json::to_string_pretty(&surface()).context("serialize the plugin API surface")?;
    text.push('\n');
    Ok(text)
}

/// The surface as a value with every object's keys in sorted order.
pub(crate) fn surface() -> Value {
    let surface = json!({
        "api_version": API_VERSION.to_string(),
        "capabilities": CAPABILITIES.iter().map(capability).collect::<Vec<_>>(),
        "effect_requests": tables(EFFECT_REQUESTS),
        "events": EVENTS,
        "filters": FILTERS,
        "host_errors": HOST_ERRORS,
        "icons": PluginIcon::ALL.map(PluginIcon::name),
        "kinds": Kind::ALL.map(Kind::name),
        "limits": LIMITS.iter().map(|(name, value)| ((*name).to_owned(), json!(value))).collect::<Map<_, _>>(),
        "manifest_tables": tables(MANIFEST_TABLES),
        "node_kinds": NodeKind::NAMES,
        "notice_tones": NoticeTone::ALL.map(NoticeTone::name),
        "refreshes": REFRESHES,
        "runtimes": RUNTIMES,
        "settings_kinds": SETTINGS_KINDS,
        "tones": Tone::ALL.map(Tone::name),
        "workspace_kinds": WorkspaceKind::ALL.map(WorkspaceKind::name),
        "zones": Zone::ALL.map(Zone::name),
    });
    canonical(surface)
}

fn tables(tables: &[(&str, &[&str])]) -> Value {
    tables
        .iter()
        .map(|(name, keys)| ((*name).to_owned(), json!(keys)))
        .collect::<Map<_, _>>()
        .into()
}

fn capability(entry: &CapabilityEntry) -> Value {
    let family = match entry.family {
        Family::Observe => "observe",
        Family::Effect => "effect",
    };
    let target = entry.target.map(|target| match target {
        EffectTarget::OwnContribution => "own-contribution",
        EffectTarget::OwnData => "own-data",
        EffectTarget::HostPresentation => "host-presentation",
    });
    let fields = fields_of(entry.id).map(|fields| {
        fields
            .iter()
            .map(|(name, ty)| ((*name).to_owned(), json!(ty.name())))
            .collect::<Map<_, _>>()
    });
    json!({
        "id": entry.id,
        "family": family,
        "doc": entry.doc,
        "consent": entry.consent,
        "excluded": entry.excluded,
        "session_derived": entry.session_derived,
        "target": target,
        "gesture": entry.gesture,
        "rate": entry.rate.map(|(uses, per)| json!({ "uses": uses, "per_ms": per.as_millis() as u64 })),
        "live_cap": entry.live_cap,
        "since": entry.since.to_string(),
        "deprecated": entry.deprecated.map(|d| json!({
            "since": d.since.to_string(),
            "successor": d.successor,
            "note": d.note,
        })),
        "fields": fields,
    })
}

/// The same value with every object's keys in sorted order, whatever the map implementation.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, canonical(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_surface_lists_every_capability_once_with_stable_keys() {
        let text = render().unwrap();
        let surface: Value = serde_json::from_str(&text).unwrap();
        let ids: Vec<&str> = surface["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), CAPABILITIES.len());
        assert!(text.ends_with("}\n"));
        assert_eq!(text, render().unwrap(), "rendering is deterministic");
        assert_eq!(surface["api_version"], API_VERSION.to_string());
        assert_eq!(
            surface["capabilities"][0]["fields"]["unix_minutes"],
            "number"
        );
    }
}
