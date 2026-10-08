//! Keeps functions small enough to read.
//!
//! The workspace warns on `clippy::too_many_lines` and
//! `clippy::cognitive_complexity`. A function already over a limit carries
//! `#[expect(..., reason = "...")]`, which fails once the function is split,
//! so the list of exceptions can only shrink. Silencing the lints with
//! `allow`, or expecting them without a reason, is rejected here.
use std::{fs, path::Path};

use anyhow::bail;

/// Lints whose exceptions must be expectations with a reason.
const GUARDED: [&str; 2] = ["too_many_lines", "cognitive_complexity"];

/// Checks every source file under `src/`, `crates/` and `xtask/`.
pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let mut files = Vec::new();
    for directory in ["src", "crates", "xtask"] {
        let directory = root.join(directory);
        if directory.is_dir() {
            crate::source_files(&directory, &mut files)?;
        }
    }
    let mut problems = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file)?;
        let name = file.strip_prefix(root).unwrap_or(&file).display();
        for (line, problem) in violations(&text) {
            problems.push(format!("{name}:{line}: {problem}"));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "complexity lints may only be expected, with a reason:\n{}",
        problems.join("\n")
    )
}

/// Problems in one file, with the line each offending attribute starts on.
fn violations(text: &str) -> Vec<(usize, &'static str)> {
    let mut problems = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        let line = line.trim_start();
        if !line.starts_with("#[") && !line.starts_with("#![") {
            continue;
        }
        // rustfmt wraps long attributes; read one up to its closing bracket.
        let mut attribute = line.to_owned();
        while depth(&attribute) > 0 {
            let Some((_, next)) = lines.next() else { break };
            attribute.push(' ');
            attribute.push_str(next.trim());
        }
        if !GUARDED.iter().any(|lint| attribute.contains(lint)) {
            continue;
        }
        if attribute.contains("allow(") {
            problems.push((index + 1, "use #[expect(..., reason = \"...\")], not allow"));
        } else if attribute.contains("expect(") && !attribute.contains("reason") {
            problems.push((index + 1, "the expectation needs a reason"));
        }
    }
    problems
}

/// How many square brackets `text` leaves open.
fn depth(text: &str) -> isize {
    text.chars().fold(0, |depth, c| match c {
        '[' => depth + 1,
        ']' => depth - 1,
        _ => depth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_expectations_with_reasons_are_accepted() {
        // `@` stands for `#`, so this file passes its own check.
        let source = "\
@[expect(clippy::too_many_lines, reason = \"split later\")]
fn a() {}
@[allow(clippy::too_many_lines)]
fn b() {}
@![allow(clippy::cognitive_complexity)]
@[expect(clippy::cognitive_complexity)]
fn c() {}
@[cfg_attr(test, allow(clippy::too_many_lines))]
fn d() {}
@[allow(clippy::needless_pass_by_value)]
fn e() {}
@[allow(
    clippy::too_many_lines,
)]
fn f() {}
@[expect(
    clippy::too_many_lines,
    reason = \"split later\"
)]
fn g() {}
"
        .replace('@', "#");
        let lines: Vec<_> = violations(&source)
            .into_iter()
            .map(|(line, _)| line)
            .collect();
        assert_eq!(lines, [3, 5, 6, 8, 12]);
    }
}
