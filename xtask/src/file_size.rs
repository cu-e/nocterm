//! Keeps source files small enough to read.
//!
//! A Rust file under `src/`, `crates/` or `xtask/` may have at most
//! [`MAX_LINES`] lines. Files that were longer when the limit was introduced
//! are listed in [`BASELINE`] with their length then; such an entry may only
//! go down, and goes away once the file is within the limit.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};
use serde::Deserialize;

/// The most lines a source file may have.
pub(crate) const MAX_LINES: usize = 800;
/// Files over the limit, relative to the repository root.
pub(crate) const BASELINE: &str = "xtask/oversized-files.toml";

#[derive(Deserialize)]
struct Baseline {
    #[serde(default)]
    files: BTreeMap<String, usize>,
}

/// Checks every source file against the limit and the baseline.
pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let text =
        fs::read_to_string(root.join(BASELINE)).with_context(|| format!("reading {BASELINE}"))?;
    let baseline: Baseline =
        toml::from_str(&text).with_context(|| format!("parsing {BASELINE}"))?;
    let mut sizes = BTreeMap::new();
    for directory in ["src", "crates", "xtask"] {
        collect(root, &root.join(directory), &mut sizes)?;
    }
    let problems = problems(&sizes, &baseline.files);
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "source files over {MAX_LINES} lines; split them into modules by responsibility:\n{}",
        problems.join("\n")
    )
}

fn collect(
    root: &Path,
    directory: &Path,
    sizes: &mut BTreeMap<String, usize>,
) -> anyhow::Result<()> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(());
    };
    for entry in entries {
        let path: PathBuf = entry?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                collect(root, &path, sizes)?;
            }
        } else if name.ends_with(".rs") {
            let text =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            sizes.insert(relative, text.lines().count());
        }
    }
    Ok(())
}

/// What is wrong with `sizes`, one line per file.
fn problems(sizes: &BTreeMap<String, usize>, baseline: &BTreeMap<String, usize>) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, &lines) in sizes {
        match baseline.get(path) {
            Some(&limit) if lines > limit => problems.push(format!(
                "  {path}: {lines} lines; it may not grow past {limit} (listed in {BASELINE})"
            )),
            Some(&limit) if lines <= MAX_LINES => problems.push(format!(
                "  {path}: {lines} lines is within the limit; remove its entry ({limit}) from {BASELINE}"
            )),
            Some(&limit) if lines < limit => problems.push(format!(
                "  {path}: shrank to {lines} lines; lower its entry in {BASELINE} from {limit} to {lines}"
            )),
            Some(_) => {}
            None if lines > MAX_LINES => {
                problems.push(format!("  {path}: {lines} lines (limit {MAX_LINES})"))
            }
            None => {}
        }
    }
    for path in baseline.keys().filter(|path| !sizes.contains_key(*path)) {
        problems.push(format!(
            "  {path}: no longer exists; remove it from {BASELINE}"
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_files_keep_to_the_limit_and_listed_ones_only_shrink() {
        let sizes = BTreeMap::from([
            ("small.rs".to_owned(), 10),
            ("new.rs".to_owned(), MAX_LINES + 1),
            ("grew.rs".to_owned(), 1001),
            ("same.rs".to_owned(), 1000),
            ("shrank.rs".to_owned(), 900),
            ("fixed.rs".to_owned(), 700),
        ]);
        let baseline = BTreeMap::from([
            ("grew.rs".to_owned(), 1000),
            ("same.rs".to_owned(), 1000),
            ("shrank.rs".to_owned(), 1000),
            ("fixed.rs".to_owned(), 1000),
            ("gone.rs".to_owned(), 1000),
        ]);
        let problems = problems(&sizes, &baseline).join("\n");
        for path in ["new.rs", "grew.rs", "shrank.rs", "fixed.rs", "gone.rs"] {
            assert!(problems.contains(path), "{path}: {problems}");
        }
        for path in ["small.rs", "same.rs"] {
            assert!(!problems.contains(path), "{path}: {problems}");
        }
    }
}
