//! The golden file of the code plugin ABI, `crates/plugin-abi/abi-golden.json`.
//!
//! It records the ABI version, the exports and the hex bytes of the canonical samples. Postcard
//! writes enum variants by index and struct fields in order, so the bytes of an existing sample
//! change when a wire type is reordered or edited in place. `cargo xtask docs` refuses to
//! rewrite a changed sample unless `ABI_VERSION` was bumped, and `--check` keeps the file fresh.
//! Appending a sample or a variant is not a change of the ABI. The committed file is the
//! reference: one that is missing or unreadable fails instead of being recreated, so deleting or
//! corrupting it cannot silence the rule.

use std::{fs, path::Path};

use anyhow::{Context as _, bail};
use nocterm_plugin_abi::{ABI_VERSION, names::EXPORTS, samples};
use serde_json::{Map, Value, json};

/// The path of the golden file, relative to the repository root.
pub(crate) const PATH: &str = "crates/plugin-abi/abi-golden.json";

/// The golden file as it should be now, or an error if it would rewrite the ABI unannounced or
/// the committed file cannot be compared.
pub(crate) fn render(root: &Path) -> anyhow::Result<String> {
    let fresh = golden();
    let committed = fs::read_to_string(root.join(PATH))
        .with_context(|| format!("read {PATH}; restore it from version control"))?;
    ensure_compatible(&committed, &fresh)?;
    let mut text = serde_json::to_string_pretty(&fresh).context("serialize the ABI golden file")?;
    text.push('\n');
    Ok(text)
}

/// The golden file of the current sources.
fn golden() -> Value {
    let exports: Map<String, Value> = EXPORTS
        .iter()
        .map(|(name, signature)| ((*name).to_owned(), json!(signature)))
        .collect();
    let samples: Map<String, Value> = samples()
        .into_iter()
        .map(|sample| (sample.name.to_owned(), json!(hex(&sample.bytes))))
        .collect();
    json!({
        "abi_version": ABI_VERSION,
        "exports": exports,
        "samples": samples,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether `fresh` may replace `committed`: the same version needs every committed export and
/// sample unchanged (new ones may be appended); a higher version replaces everything. A committed
/// file that is not JSON or names no version is an error, and so is a lower version.
fn ensure_compatible(committed: &str, fresh: &Value) -> anyhow::Result<()> {
    let committed = serde_json::from_str::<Value>(committed)
        .with_context(|| format!("{PATH} is not valid JSON; restore it from version control"))?;
    let version = |value: &Value| value["abi_version"].as_u64();
    let Some(committed_version) = version(&committed) else {
        bail!("{PATH} names no abi_version; restore it from version control");
    };
    let fresh_version = version(fresh);
    if fresh_version > Some(committed_version) {
        return Ok(());
    }
    if fresh_version < Some(committed_version) {
        bail!("ABI_VERSION went down from {committed_version}; the version only grows");
    }
    for table in ["exports", "samples"] {
        for (name, old) in committed[table].as_object().into_iter().flatten() {
            if fresh[table].get(name) != Some(old) {
                bail!(
                    "{table} `{name}` of the plugin ABI changed (or was removed) but ABI_VERSION is still {}: \
                     bump ABI_VERSION in crates/plugin-abi/src/lib.rs, or revert the change to the wire types",
                    ABI_VERSION
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(version: u64, samples: &[(&str, &str)]) -> String {
        let samples: Map<String, Value> = samples
            .iter()
            .map(|(name, hex)| ((*name).to_owned(), json!(hex)))
            .collect();
        json!({ "abi_version": version, "exports": {"nocterm_call": "(i32, i32) -> i64"}, "samples": samples })
            .to_string()
    }

    fn fresh(version: u64, samples: &[(&str, &str)]) -> Value {
        serde_json::from_str(&text(version, samples)).unwrap()
    }

    #[test]
    fn the_golden_file_is_deterministic_and_lists_the_samples() {
        let first = golden();
        assert_eq!(first, golden());
        assert_eq!(first["abi_version"], json!(ABI_VERSION));
        let listed = first["samples"].as_object().unwrap();
        assert_eq!(listed.len(), samples().len());
        assert!(
            listed
                .values()
                .all(|hex| hex.as_str().is_some_and(|h| !h.is_empty()))
        );
        assert!(first["exports"]["nocterm_call"].is_string());
    }

    #[test]
    fn an_unchanged_or_appended_sample_is_accepted() {
        let committed = text(1, &[("a", "00"), ("b", "01")]);
        assert!(ensure_compatible(&committed, &fresh(1, &[("a", "00"), ("b", "01")])).is_ok());
        assert!(
            ensure_compatible(
                &committed,
                &fresh(1, &[("a", "00"), ("b", "01"), ("c", "02")])
            )
            .is_ok()
        );
    }

    #[test]
    fn a_changed_or_removed_sample_needs_a_new_version() {
        let committed = text(1, &[("a", "00"), ("b", "01")]);
        let changed = ensure_compatible(&committed, &fresh(1, &[("a", "00"), ("b", "ff")]));
        assert!(
            changed
                .unwrap_err()
                .to_string()
                .contains("bump ABI_VERSION")
        );
        assert!(ensure_compatible(&committed, &fresh(1, &[("a", "00")])).is_err());
        assert!(ensure_compatible(&committed, &fresh(2, &[("a", "ff")])).is_ok());
        let lowered = ensure_compatible(&text(2, &[("a", "00")]), &fresh(1, &[("a", "00")]));
        assert!(lowered.unwrap_err().to_string().contains("went down"));
    }

    #[test]
    fn a_missing_or_unparsable_committed_file_fails_instead_of_being_replaced() {
        let fresh = fresh(1, &[("a", "00")]);
        assert!(ensure_compatible("not json", &fresh).is_err());
        assert!(ensure_compatible("{}", &fresh).is_err());
        assert!(ensure_compatible("", &fresh).is_err());
        let missing = render(Path::new("/nonexistent/nocterm-root"));
        assert!(missing.unwrap_err().to_string().contains("restore it"));
    }

    #[test]
    fn a_changed_export_needs_a_new_version() {
        let committed = text(1, &[]);
        let mut changed = fresh(1, &[]);
        changed["exports"]["nocterm_call"] = json!("(i32) -> i64");
        assert!(ensure_compatible(&committed, &changed).is_err());
    }

    #[test]
    fn the_committed_golden_file_matches_the_sources() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let committed = fs::read_to_string(root.join(PATH)).unwrap();
        assert_eq!(committed, render(root).unwrap(), "run cargo xtask docs");
    }
}
