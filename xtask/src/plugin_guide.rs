//! The check of the hand-written plugin documents, run with `cargo xtask docs [--check]`.
//!
//! A capability id written as inline code (`clock.now`) must be a capability the contract
//! defines, and an id the contract has deprecated may be named only in a paragraph that says it
//! is deprecated. Fenced code and file names are not inspected. The generated reference is not
//! checked: it is written from the contract.

use std::{collections::BTreeSet, fmt, fs, path::Path};

use anyhow::Context as _;

/// The directory of the plugin documents, relative to the repository root.
const DIRECTORY: &str = "docs/plugins";
/// The generated document, which is not checked.
const REFERENCE: &str = "reference.md";
/// Endings that make a dotted word a file name.
const EXTENSIONS: [&str; 7] = ["json", "toml", "md", "rs", "wasm", "sh", "txt"];

/// What is wrong with a name in a document.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GuideError {
    /// The name looks like a capability id, and no capability has it.
    UnknownId { line: usize, name: String },
    /// The id is deprecated and its paragraph does not say so.
    DeprecatedWithoutNote { line: usize, name: String },
}

impl fmt::Display for GuideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownId { line, name } => {
                write!(f, "line {line}: `{name}` is not a capability id")
            }
            Self::DeprecatedWithoutNote { line, name } => write!(
                f,
                "line {line}: `{name}` is deprecated; its paragraph must say \"deprecated\""
            ),
        }
    }
}

/// The problems in `text`. `ids` are the names that may be written as inline code in the shape of
/// a capability id; `deprecated` are those of them the contract has deprecated.
pub(crate) fn check(
    text: &str,
    ids: &BTreeSet<&str>,
    deprecated: &BTreeSet<&str>,
) -> Vec<GuideError> {
    let mut errors = Vec::new();
    let mut fenced = false;
    // The names of the paragraph so far, with the line each was found on.
    let mut names: Vec<(usize, String)> = Vec::new();
    let mut says_deprecated = false;
    let mut close = |names: &mut Vec<(usize, String)>, says: &mut bool| {
        for (line, name) in names.drain(..) {
            if !ids.contains(name.as_str()) {
                errors.push(GuideError::UnknownId { line, name });
            } else if deprecated.contains(name.as_str()) && !*says {
                errors.push(GuideError::DeprecatedWithoutNote { line, name });
            }
        }
        *says = false;
    };
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if line.trim().is_empty() {
            close(&mut names, &mut says_deprecated);
            continue;
        }
        says_deprecated |= line.to_ascii_lowercase().contains("deprecated");
        for span in spans(line) {
            if is_id_shaped(span) {
                names.push((index + 1, span.to_owned()));
            }
        }
    }
    close(&mut names, &mut says_deprecated);
    errors
}

/// The single-backtick code spans of one line.
fn spans(line: &str) -> impl Iterator<Item = &str> {
    line.split('`').skip(1).step_by(2)
}

/// `family.name`, lower case, and not a file name.
fn is_id_shaped(span: &str) -> bool {
    let Some((family, name)) = span.split_once('.') else {
        return false;
    };
    let family_ok = family.starts_with(|c: char| c.is_ascii_lowercase())
        && family
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let name_ok = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    family_ok && name_ok && !EXTENSIONS.contains(&name)
}

/// Checks every hand-written document of the plugin directory against the contract.
pub(crate) fn check_documents(root: &Path) -> anyhow::Result<()> {
    let mut ids: BTreeSet<&str> = nocterm_plugin_api::CAPABILITIES
        .iter()
        .map(|entry| entry.id)
        .collect();
    let deprecated: BTreeSet<&str> = nocterm_plugin_api::CAPABILITIES
        .iter()
        .filter(|entry| entry.deprecated.is_some())
        .map(|entry| entry.id)
        .collect();
    // The name of the custom section of a code package is written the same way.
    ids.insert(nocterm_plugin_api::MANIFEST_SECTION);
    let mut problems = Vec::new();
    let directory = root.join(DIRECTORY);
    let mut files: Vec<_> = fs::read_dir(&directory)
        .with_context(|| format!("read {}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "md")
                && path.file_name().is_some_and(|name| name != REFERENCE)
        })
        .collect();
    files.sort();
    for path in files {
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        for error in check(&text, &ids, &deprecated) {
            problems.push(format!("{}: {error}", path.display()));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "the plugin documents name capabilities wrongly:\n{}",
            problems.join("\n")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Vec<GuideError> {
        let ids = BTreeSet::from(["clock.now", "monitor.usage", "old.reading"]);
        let deprecated = BTreeSet::from(["old.reading"]);
        check(text, &ids, &deprecated)
    }

    #[test]
    fn a_known_id_in_a_code_span_passes() {
        assert_eq!(run("Ask for `clock.now` and `monitor.usage`.\n"), []);
    }

    #[test]
    fn an_unknown_id_is_reported_with_its_line() {
        assert_eq!(
            run("first\n\nAsk for `clock.nwo`.\n"),
            [GuideError::UnknownId {
                line: 3,
                name: "clock.nwo".into()
            }]
        );
    }

    #[test]
    fn a_deprecated_id_needs_the_word_in_its_paragraph() {
        assert_eq!(
            run("Use `old.reading`.\n"),
            [GuideError::DeprecatedWithoutNote {
                line: 1,
                name: "old.reading".into()
            }]
        );
        assert_eq!(run("Use `old.reading`.\nIt is Deprecated since 0.2.\n"), []);
        assert_eq!(
            run("The word is in another paragraph: deprecated.\n\nUse `old.reading`.\n"),
            [GuideError::DeprecatedWithoutNote {
                line: 3,
                name: "old.reading".into()
            }]
        );
    }

    #[test]
    fn fenced_code_file_names_and_other_shapes_are_not_inspected() {
        let text = "```toml\ncapabilities = [\"clock.nwo\"]\n```\n\n\
                    See `plugin.toml`, `reference.md`, `cx.log(..)`, `Clock.Now`, `a.b.c` and `x`.\n";
        assert_eq!(run(text), []);
    }
}
