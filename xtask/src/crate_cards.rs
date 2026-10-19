//! Every crate under `crates/` starts with an ownership card in its root documentation.
//!
//! The card is the boundary a design step checks new code against: what the
//! crate owns, what it does not, the ports it uses or provides, its settings
//! section, globals and invariants. This is a presence check on the crate-level
//! `//!` documentation, not a judgement of the content.
use anyhow::{Context as _, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{Expr, Lit, Meta};

/// Labels a card must contain. `Ports`, `Settings`, `Globals` and `Invariant`
/// may carry a qualifier before the colon (`Ports used:`, `Settings section:`).
pub(crate) const LABELS: [&str; 6] = [
    "Owns:",
    "Does not own:",
    "Ports",
    "Settings",
    "Globals",
    "Invariant",
];

/// Longest qualifier accepted between a label and its colon.
const QUALIFIER: usize = 24;

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    for package in crate::metadata(root)?.packages {
        if !package.manifest_path.starts_with(root.join("crates")) {
            continue;
        }
        let file = root_file(&package).context("package without a library or binary target")?;
        let missing = missing(&fs::read_to_string(&file)?)
            .with_context(|| format!("parse {}", file.display()))?;
        if !missing.is_empty() {
            problems.push(format!(
                "  {} ({}): {}",
                package.name,
                file.strip_prefix(root).unwrap_or(&file).display(),
                missing.join(", ")
            ));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "crate cards are missing labels; start the crate documentation with an ownership card (AGENTS.md, Documents):\n{}",
        problems.join("\n")
    )
}

/// The crate root that carries the card: the library, else the first binary.
pub(crate) fn root_file(package: &crate::Package) -> Option<PathBuf> {
    ["lib", "proc-macro", "bin"].into_iter().find_map(|kind| {
        package
            .targets
            .iter()
            .find(|target| target.kind.iter().any(|candidate| candidate == kind))
            .map(|target| target.src_path.clone())
    })
}

/// Labels absent from the crate-level documentation of `source`.
pub(crate) fn missing(source: &str) -> anyhow::Result<Vec<&'static str>> {
    let documentation = documentation(&syn::parse_file(source)?);
    Ok(LABELS
        .into_iter()
        .filter(|label| !labelled(&documentation, label))
        .collect())
}

fn documentation(file: &syn::File) -> String {
    file.attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("doc"))
        .filter_map(|attribute| match &attribute.meta {
            Meta::NameValue(value) => match &value.value {
                Expr::Lit(literal) => match &literal.lit {
                    Lit::Str(text) => Some(text.value()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn labelled(documentation: &str, label: &str) -> bool {
    if label.ends_with(':') {
        return documentation.contains(label);
    }
    documentation.match_indices(label).any(|(index, _)| {
        let rest = &documentation[index + label.len()..];
        rest.find(':')
            .is_some_and(|colon| colon <= QUALIFIER && !rest[..colon].contains(['.', '\n', '`']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_cards_accept_qualified_labels_across_paragraphs() {
        let source = "//! Containers on a host.\n//!\n//! Owns: engine detection.\n//! Does not own: views.\n//! Ports used: HostExec. Ports provided: none. Settings section: none. Globals: none.\n\n//! Invariants: listing output is bounded.\nmod engine;\n";
        assert!(missing(source).unwrap().is_empty());
        let singular = "//! Owns: a.\n//! Does not own: b.\n//! Ports: c. Settings: d. Globals: none.\n//! Invariant: e.\n";
        assert!(missing(singular).unwrap().is_empty());
    }

    #[test]
    fn prose_mentions_do_not_count_as_labels() {
        let source = "//! Settings are read here. The Ports module. Globals are not used.\n//! Owns: a.\n//! Does not own: b.\nfn main() {}\n";
        assert_eq!(
            missing(source).unwrap(),
            ["Ports", "Settings", "Globals", "Invariant"]
        );
    }

    #[test]
    fn item_documentation_is_not_the_crate_card() {
        let source = "/// Owns: a. Does not own: b. Ports: c. Settings: d. Globals: e. Invariant: f.\npub struct Model;\n";
        assert_eq!(missing(source).unwrap(), LABELS);
    }
}
