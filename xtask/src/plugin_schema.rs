//! The JSON Schema of a plugin manifest, `docs/plugins/manifest.schema.json`.
//!
//! It is written from the typed manifest keys of the API surface (JSON Schema 2020-12), so an
//! editor that validates TOML against a schema offers the keys the host reads and flags the
//! rest. Limits stay in the reference: the schema says what a key holds, not how long it may be.

use anyhow::Context as _;
use serde_json::{Map, Value, json};

/// The path of the schema, relative to the repository root.
pub(crate) const PATH: &str = "docs/plugins/manifest.schema.json";

pub(crate) fn render() -> anyhow::Result<String> {
    let mut text = serde_json::to_string_pretty(&schema(&crate::plugin_reference::surface()))
        .context("serialize the manifest schema")?;
    text.push('\n');
    Ok(text)
}

/// The schema of the `manifest` table of `surface`, with one definition per table.
fn schema(surface: &Value) -> Value {
    let mut definitions = Map::new();
    for (table, keys) in surface["manifest"].as_object().into_iter().flatten() {
        definitions.insert(table.clone(), definition(table, keys, surface));
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Nocterm user plugin manifest",
        "description": "The TOML a user plugin package carries. Unknown keys are refused by the host.",
        "$ref": "#/$defs/manifest",
        "$defs": definitions,
    })
}

fn definition(table: &str, keys: &Value, surface: &Value) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, entry) in keys.as_object().into_iter().flatten() {
        properties.insert(name.clone(), property(table, name, entry, surface));
        if entry["required"] == true {
            required.push(json!(name));
        }
    }
    let mut definition = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    });
    if !required.is_empty() {
        definition["required"] = Value::Array(required);
    }
    definition
}

/// The table a `table` or `table-list` key holds: the key under its parent's path.
fn child(table: &str, key: &str) -> String {
    if table == "manifest" {
        key.to_owned()
    } else {
        format!("{table}.{key}")
    }
}

fn property(table: &str, key: &str, entry: &Value, surface: &Value) -> Value {
    match entry["type"].as_str().unwrap_or_default() {
        "text" => json!({ "type": "string" }),
        "integer" => json!({
            "type": "integer",
            "minimum": entry["min"],
            "maximum": entry["max"],
        }),
        "bool" => json!({ "type": "boolean" }),
        "text-list" => json!({ "type": "array", "items": { "type": "string" } }),
        "text-map" => json!({ "type": "object", "additionalProperties": { "type": "string" } }),
        "table" => json!({ "$ref": format!("#/$defs/{}", child(table, key)) }),
        "table-list" => json!({
            "type": "array",
            "items": { "$ref": format!("#/$defs/{}", child(table, key)) },
        }),
        "choice" => {
            let vocabulary = entry["vocabulary"].as_str().unwrap_or_default();
            json!({ "enum": surface[vocabulary] })
        }
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    /// The part of JSON Schema the generated file uses: `type`, `enum`, `required`, `properties`,
    /// `additionalProperties`, `items`, `minimum`, `maximum` and local `$ref`.
    fn valid(value: &Value, schema: &Value, root: &Value) -> bool {
        if let Some(target) = schema["$ref"].as_str() {
            let name = target.trim_start_matches("#/$defs/");
            return valid(value, &root["$defs"][name], root);
        }
        if let Some(options) = schema.get("enum") {
            return options.as_array().is_some_and(|o| o.contains(value));
        }
        match schema.get("type").and_then(Value::as_str) {
            None => true,
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("integer") => value.as_i64().is_some_and(|number| {
                schema["minimum"]
                    .as_i64()
                    .is_none_or(|least| number >= least)
                    && schema["maximum"].as_i64().is_none_or(|most| number <= most)
            }),
            Some("array") => value
                .as_array()
                .is_some_and(|items| items.iter().all(|item| valid(item, &schema["items"], root))),
            Some("object") => {
                let Some(object) = value.as_object() else {
                    return false;
                };
                let properties = schema["properties"].as_object();
                let extra = &schema["additionalProperties"];
                let fits =
                    object
                        .iter()
                        .all(|(key, item)| match properties.and_then(|p| p.get(key)) {
                            Some(property) => valid(item, property, root),
                            None if *extra == false => false,
                            None if extra.is_object() => valid(item, extra, root),
                            None => true,
                        });
                let present = schema["required"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .all(|key| object.contains_key(key));
                fits && present
            }
            Some(other) => panic!("the test validator does not know {other}"),
        }
    }

    fn generated() -> Value {
        serde_json::from_str(&render().unwrap()).unwrap()
    }

    fn manifest(text: &str) -> Value {
        toml::from_str::<Value>(text).expect("the manifest is TOML")
    }

    #[test]
    fn the_schema_accepts_every_example_manifest() {
        let schema = generated();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/plugins");
        let mut checked = 0;
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            let file = if path.is_dir() {
                path.join("plugin.toml")
            } else {
                path.clone()
            };
            if file.extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let value = manifest(&fs::read_to_string(&file).unwrap());
            assert!(valid(&value, &schema, &schema), "{}", file.display());
            checked += 1;
        }
        assert!(checked >= 3, "the clock, the notes and the focus timer");
    }

    #[test]
    fn the_schema_refuses_an_unknown_key_a_missing_required_key_and_a_wrong_form() {
        let schema = generated();
        let ok = "api = \"0.1\"\npublisher = \"a\"\nid = \"b\"\nname = \"N\"\n";
        assert!(valid(&manifest(ok), &schema, &schema));
        for broken in [
            format!("{ok}brand-new = true\n"),
            "api = \"0.1\"\npublisher = \"a\"\nid = \"b\"\n".to_owned(),
            format!("{ok}capabilities = \"clock.now\"\n"),
            format!("{ok}runtime = \"python\"\n"),
            format!(
                "{ok}[[contribution]]\nlocal = \"x\"\nkind = \"widget\"\ntitle = \"T\"\nzone = \"up\"\n"
            ),
            format!(
                "{ok}[[contribution]]\nlocal = \"x\"\nkind = \"widget\"\ntitle = \"T\"\nbogus = 1\n"
            ),
        ] {
            assert!(!valid(&manifest(&broken), &schema, &schema), "{broken}");
        }
    }

    #[test]
    fn the_schema_takes_a_negative_number_setting_and_refuses_what_the_host_would() {
        let schema = generated();
        let ok = "api = \"0.1\"\npublisher = \"a\"\nid = \"b\"\nname = \"N\"\n";
        let setting = |fields: &str| {
            format!(
                "{ok}[settings]\n[[settings.group]]\ntitle = \"G\"\n[[settings.group.field]]\n\
                 key = \"k\"\nlabel = \"L\"\nkind = \"number\"\n{fields}"
            )
        };
        let offset = setting("min = -12\nmax = 14\nstep = 1\n");
        assert!(valid(&manifest(&offset), &schema, &schema), "a UTC offset");
        for broken in [
            setting("min = 0\nmax = 10\nstep = 0\n"),
            setting("min = 0\nmax = 10\nstep = -1\n"),
            setting("min = -3000000000\nmax = 10\n"),
            setting("min = 0\nmax = 10\nmax_len = 70000\n"),
        ] {
            assert!(!valid(&manifest(&broken), &schema, &schema), "{broken}");
        }
    }

    #[test]
    fn rendering_is_deterministic_and_lists_every_table() {
        let text = render().unwrap();
        assert_eq!(text, render().unwrap());
        let schema = generated();
        assert_eq!(
            schema["$defs"].as_object().unwrap().len(),
            crate::plugin_reference::surface()["manifest"]
                .as_object()
                .unwrap()
                .len()
        );
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }
}
