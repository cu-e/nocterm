//! Checks maintained prose, excluding historical accepted decisions and code examples.
use super::symbols::Symbols;
use anyhow::bail;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn check(root: &Path, symbols: &Symbols) -> anyhow::Result<()> {
    let mut files = Vec::new();
    documents(&root.join("docs"), &mut files)?;
    let mut problems = Vec::new();
    for file in files {
        if file.starts_with(root.join("docs/adr")) || file.starts_with(root.join("docs/history")) {
            continue;
        }
        let text = fs::read_to_string(&file)?;
        for (line, problem) in violations(&text, symbols) {
            problems.push(format!(
                "{}:{line}: {problem}",
                file.strip_prefix(root)?.display()
            ));
        }
    }
    if !problems.is_empty() {
        bail!("documentation hygiene:\n{}", problems.join("\n"));
    }
    Ok(())
}

fn documents(directory: &Path, paths: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            documents(&path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "md") {
            paths.push(path);
        }
    }
    Ok(())
}

pub(super) fn violations(text: &str, symbols: &Symbols) -> Vec<(usize, String)> {
    let mut problems = Vec::new();
    let mut fence = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if [
            "the user authorized",
            "the user explicitly authorized",
            "status: approved",
            "underway",
        ]
        .iter()
        .any(|phrase| lower.contains(phrase))
        {
            problems.push((
                index + 1,
                "session status belongs in .claude/plans/, not docs/".into(),
            ));
        }
        for token in line.split('`').skip(1).step_by(2) {
            let reference = token.strip_suffix("()").unwrap_or(token);
            if !reference.contains("::")
                || !reference
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
            {
                continue;
            }
            if let Some(problem) = symbol_reference(reference, symbols) {
                problems.push((index + 1, problem));
            }
        }
    }
    problems
}

fn symbol_reference(reference: &str, symbols: &Symbols) -> Option<String> {
    if reference.starts_with("nocterm_") {
        let root = reference.split("::").next()?;
        let path = syn::parse_str::<syn::Path>(reference).ok()?;
        let resolved = symbols.resolve(root, &path, &std::collections::BTreeMap::new());
        if symbols.contains(&resolved) {
            return None;
        }
        if let Some((owner, member)) = resolved.rsplit_once("::")
            && symbols
                .members
                .get(owner)
                .is_some_and(|members| members.contains(member))
        {
            return None;
        }
        return Some(format!("unknown repository Rust symbol {reference}"));
    }
    let (owner, member) = reference.split_once("::")?;
    if owner
        .chars()
        .next()
        .is_none_or(|first| !first.is_uppercase())
    {
        return None;
    }
    let candidates: Vec<_> = symbols
        .members
        .iter()
        .filter(|(name, _)| name.rsplit("::").next() == Some(owner))
        .collect();
    // External APIs and placeholders have no repository declaration.
    if candidates.is_empty()
        || candidates
            .iter()
            .any(|(_, members)| members.contains(member))
    {
        return None;
    }
    Some(format!(
        "unknown member in repository Rust symbol {reference}"
    ))
}
