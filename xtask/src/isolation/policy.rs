//! The reviewed isolation policy: closed lists, each exemption with a reason.
use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Context as _;
use serde::Deserialize;

/// How a row's package is held to the `std` rules.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Kind {
    /// Domain crate: `#![no_std]` with `alloc`, so `std` is unreachable (I2).
    NoStd,
    /// Uses `std`; I/O paths are forbidden outside reviewed exceptions (I2b).
    Std,
    /// Uses `std` and gpui; additionally bound to the gpui receiver rule (I3).
    Gpui,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Row {
    pub(super) kind: Kind,
    /// Full transitive `nocterm-*` closure.
    pub(super) closure: Vec<String>,
    /// Direct external dependencies.
    pub(super) direct: Vec<String>,
    /// Names of the external lists that bound the external closure.
    pub(super) external: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Providers {
    pub(super) allowed: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompileOut {
    pub(super) crates: Vec<String>,
    #[serde(default)]
    pub(super) features: BTreeMap<String, String>,
    /// Packages whose named feature may be enabled by tests only: it must be absent from the
    /// normal (non-dev) closure of the app with the user-plugins feature.
    #[serde(default)]
    pub(super) test_features: BTreeMap<String, String>,
    /// Packages whose named feature no build of the workspace may switch on, whoever asks for
    /// it: `wasmi` with `auto-dispatch` overflowed the stack at the optimization level the
    /// workspace builds it with.
    #[serde(default)]
    pub(super) banned_features: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identifiers {
    pub(super) forbidden: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    pub(super) gpui: String,
    pub(super) app: String,
    /// Crates the application never reaches, with or without the user-plugins feature.
    #[serde(default)]
    pub(super) guest_only: Vec<String>,
    #[serde(default)]
    pub(super) external: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub(super) reviewed: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) crates: BTreeMap<String, Row>,
    #[serde(default)]
    pub(super) io_exceptions: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) gpui_allowed: BTreeMap<String, String>,
    pub(super) providers: Providers,
    pub(super) compile_out: CompileOut,
    #[serde(default)]
    pub(super) identifiers: BTreeMap<String, Identifiers>,
}

#[derive(Deserialize)]
struct File {
    isolation: Policy,
}

impl Policy {
    pub(super) fn read(path: &Path) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
        Self::parse(&text)
    }

    pub(super) fn parse(text: &str) -> anyhow::Result<Self> {
        let policy = toml::from_str::<File>(text)
            .context("[isolation] policy")?
            .isolation;
        policy.validate()?;
        Ok(policy)
    }

    fn validate(&self) -> anyhow::Result<()> {
        for (kind, table) in [
            ("reviewed", &self.reviewed),
            ("io_exceptions", &self.io_exceptions),
            ("gpui_allowed", &self.gpui_allowed),
        ] {
            if let Some((name, _)) = table.iter().find(|(_, reason)| reason.trim().is_empty()) {
                anyhow::bail!("isolation {kind} entry {name} needs a reason");
            }
        }
        for (name, row) in &self.crates {
            if let Some(list) = row
                .external
                .iter()
                .find(|l| !self.external.contains_key(*l))
            {
                anyhow::bail!("isolation row {name} names unknown external list {list}");
            }
            if let Some(dependency) = row.closure.iter().find(|c| !c.starts_with("nocterm-")) {
                anyhow::bail!("isolation row {name}: closure entry {dependency} is not internal");
            }
        }
        for name in self.reviewed.keys() {
            if !self.external.values().any(|list| list.contains(name)) {
                anyhow::bail!("isolation reviewed entry {name} is in no external list");
            }
        }
        Ok(())
    }

    /// External crates a row may reach: the union of its named lists.
    pub(super) fn external_for(&self, row: &Row) -> std::collections::BTreeSet<&str> {
        row.external
            .iter()
            .filter_map(|list| self.external.get(list))
            .flatten()
            .map(String::as_str)
            .collect()
    }
}
