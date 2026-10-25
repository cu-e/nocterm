//! The reference of the user plugin API for plugin authors, `docs/plugins/reference.md`.
//!
//! It is written from the same surface as `api-surface.json`, so the names, limits and consent
//! texts an author reads are the ones the host enforces.

use std::fmt::Write as _;

use serde_json::Value;

/// The path of the reference, relative to the repository root.
pub(crate) const PATH: &str = "docs/plugins/reference.md";

pub(crate) fn render() -> anyhow::Result<String> {
    Ok(markdown(&crate::plugin_reference::surface()))
}

fn markdown(surface: &Value) -> String {
    let mut text = String::from(crate::GENERATED);
    let _ = write!(
        text,
        "# Plugin API reference\n\nPreview. This is the contract of user plugins at API {}, declarative packages and code packages. \
         Read the [guide](guide.md) first. Names and limits below are enforced by the host; \
         a package that breaks one is refused with the reason.\n\n",
        surface["api_version"].as_str().unwrap_or_default()
    );
    capabilities(&mut text, surface);
    manifest(&mut text, surface);
    vocabulary(&mut text, surface);
    limits(&mut text, surface);
    code_abi(&mut text);
    text
}

fn capabilities(text: &mut String, surface: &Value) {
    text.push_str(
        "## Capabilities\n\nA package lists the capabilities it needs in `capabilities` \
         (required) and `optional_capabilities`. The user sees the sentence in the Consent \
         column before installing and decides about each optional one then; a single \
         capability cannot be revoked later. A plugin never sees what a capability excludes.\n\n\
         | Id | Kind | Since | Consent sentence | Not included | Result fields |\n\
         | --- | --- | --- | --- | --- | --- |\n",
    );
    for entry in surface["capabilities"].as_array().into_iter().flatten() {
        let fields = entry["fields"]
            .as_object()
            .map_or_else(String::new, |fields| {
                fields
                    .iter()
                    .map(|(name, ty)| format!("`{name}`: {}", ty.as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
        let _ = writeln!(
            text,
            "| `{}` | {} | {} | {} | {} | {} |",
            entry["id"].as_str().unwrap_or_default(),
            entry["family"].as_str().unwrap_or_default(),
            entry["since"].as_str().unwrap_or_default(),
            cell(&entry["consent"]),
            cell(&entry["excluded"]),
            fields,
        );
    }
    text.push('\n');
}

fn manifest(text: &mut String, surface: &Value) {
    text.push_str(
        "## Manifest keys\n\nA package is one TOML file. Unknown keys are refused. \
         `contribution` and `settings.group` are arrays of tables.\n\n| Table | Keys |\n| --- | --- |\n",
    );
    for (table, keys) in surface["manifest_tables"].as_object().into_iter().flatten() {
        let _ = writeln!(text, "| `{table}` | {} |", names(keys));
    }
    text.push_str("\n### Effect requests\n\nOnly a runtime that runs code returns effects; \
         a declarative package cannot. The host lists them so that the capability table stays one list.\n\n\
         | Effect | Fields |\n| --- | --- |\n");
    for (effect, keys) in surface["effect_requests"].as_object().into_iter().flatten() {
        let _ = writeln!(text, "| `{effect}` | {} |", names(keys));
    }
    text.push('\n');
}

fn vocabulary(text: &mut String, surface: &Value) {
    text.push_str("## Vocabulary\n\n| Name | Values |\n| --- | --- |\n");
    for (title, key) in [
        ("Contribution kinds", "kinds"),
        ("Zones", "zones"),
        ("Icons", "icons"),
        ("View node kinds", "node_kinds"),
        ("Tones", "tones"),
        ("Notice tones", "notice_tones"),
        ("Refresh forms", "refreshes"),
        ("Template filters", "filters"),
        ("Events", "events"),
        ("Host errors", "host_errors"),
        ("Settings field kinds", "settings_kinds"),
        ("Workspace kinds", "workspace_kinds"),
        ("Runtimes", "runtimes"),
    ] {
        let _ = writeln!(text, "| {title} | {} |", names(&surface[key]));
    }
    text.push('\n');
}

fn limits(text: &mut String, surface: &Value) {
    text.push_str("## Limits\n\n| Limit | Value |\n| --- | --- |\n");
    for (name, value) in surface["limits"].as_object().into_iter().flatten() {
        let _ = writeln!(text, "| `{name}` | {value} |");
    }
}

/// The ABI of a code package, from the constants of `nocterm-plugin-abi`.
fn code_abi(text: &mut String) {
    use nocterm_plugin_abi::{
        ABI_VERSION, CALL_FUEL, MAX_INPUT_BYTES, MAX_LIVE_INSTANCES, MAX_LOG_LINES,
        MAX_REPLY_BYTES, MEMORY_BYTES, START_FUEL, names::EXPORTS,
    };
    use nocterm_plugin_api::{MANIFEST_SECTION, MAX_MODULE_BYTES};
    let _ = write!(
        text,
        "\n## Code package ABI\n\nA code package is one WebAssembly module (at most {MAX_MODULE_BYTES} bytes) \
         whose custom section `{MANIFEST_SECTION}` holds the manifest TOML, with `runtime = \"wasm\"`. \
         The module has no imports: it cannot reach the host except through the call below. \
         ABI version: {ABI_VERSION}. The golden file `crates/plugin-abi/abi-golden.json` pins the \
         bytes of the wire types, and the host refuses a module whose `nocterm_abi_version` differs.\n\n\
         | Export | Signature |\n| --- | --- |\n"
    );
    for (name, signature) in EXPORTS {
        let _ = writeln!(text, "| `{name}` | `{signature}` |");
    }
    let _ = write!(
        text,
        "\nThe host writes a postcard `Call` into a buffer from `nocterm_alloc` and calls \
         `nocterm_call(ptr, len)`; the guest returns `ptr << 32 | len` of a postcard `Reply`. \
         The reply pointer stays valid until the next call.\n\n\
         | ABI limit | Value |\n| --- | --- |\n\
         | Call input | {MAX_INPUT_BYTES} bytes |\n\
         | Reply | {MAX_REPLY_BYTES} bytes |\n\
         | Log lines per call | at most {MAX_LOG_LINES} |\n\
         | Fuel per call | {CALL_FUEL} |\n\
         | Fuel to instantiate | {START_FUEL} |\n\
         | Linear memory per instance | {MEMORY_BYTES} bytes |\n\
         | Live instances | {MAX_LIVE_INSTANCES} |\n"
    );
}

/// A list of names as inline code, comma separated.
fn names(list: &Value) -> String {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A sentence of a table cell: `|` would end the cell.
fn cell(value: &Value) -> String {
    value.as_str().unwrap_or("none").replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_names_every_capability_and_is_deterministic() {
        let text = render().unwrap();
        for entry in nocterm_plugin_api::CAPABILITIES {
            assert!(text.contains(&format!("`{}`", entry.id)), "{}", entry.id);
        }
        assert!(text.starts_with(crate::GENERATED));
        assert_eq!(text, render().unwrap());
    }
}
