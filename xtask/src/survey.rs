//! `cargo xtask survey`: the structural facts a design step needs about one crate.
//!
//! The survey reports file sizes, struct shapes, implementation spread, module
//! references, long tests and early warnings below the enforced limits. With
//! `--type` it adds a method × field matrix and candidate state owners. It is
//! a syntactic report for planning, not a check: names are keyed by their last
//! path segment and macros are not expanded.
use anyhow::{Context as _, bail};
use std::{fs, io::Write as _, path::Path};

mod facts;
mod matrix;
mod render;
mod report;
#[cfg(test)]
mod tests;

use report::{Source, Survey};

const USAGE: &str = "usage: cargo xtask survey <package> [--json] [--type <Type>]";

pub(crate) fn run(root: &Path, arguments: &[String]) -> anyhow::Result<()> {
    let mut package = None;
    let mut json = false;
    let mut owner = None;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--type" => owner = Some(arguments.next().context(USAGE)?.clone()),
            name if package.is_none() && !name.starts_with('-') => package = Some(name),
            _ => bail!(USAGE),
        }
    }
    let survey = survey(root, package.context(USAGE)?, owner.as_deref())?;
    let mut output = std::io::stdout().lock();
    if json {
        serde_json::to_writer_pretty(&mut output, &survey)?;
        writeln!(output)?;
    } else {
        output.write_all(render::text(&survey).as_bytes())?;
    }
    Ok(())
}

fn survey(root: &Path, name: &str, owner: Option<&str>) -> anyhow::Result<Survey> {
    let metadata = crate::metadata(root)?;
    let package = metadata
        .packages
        .iter()
        .find(|package| {
            package.name == name
                || package.name == format!("nocterm-{name}")
                || package
                    .manifest_path
                    .parent()
                    .is_some_and(|directory| directory.ends_with(Path::new("crates").join(name)))
        })
        .with_context(|| format!("no workspace package named {name}"))?;
    let directory = package
        .manifest_path
        .parent()
        .context("package directory")?;
    let layer = package
        .metadata
        .pointer("/nocterm/layer")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("undeclared")
        .to_owned();
    let sources = sources(root, directory)?;
    let mut survey = report::summarize(package.name.clone(), layer, &sources);
    if let Some(owner) = owner {
        survey.matrix = Some(type_matrix(owner, &sources)?);
    }
    Ok(survey)
}

/// Non-failing summary for `cargo xtask architecture`: one line per package
/// that has reached an early-warning threshold.
pub(crate) fn warnings(root: &Path) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    for package in crate::metadata(root)?.packages {
        let directory = package
            .manifest_path
            .parent()
            .context("package directory")?;
        if !first_party(root, directory) {
            continue;
        }
        let sources = sources(root, directory)?;
        let survey = report::summarize(package.name.clone(), String::new(), &sources);
        if let Some(line) = render::warning_summary(&survey) {
            lines.push(line);
        }
    }
    Ok(lines)
}

fn first_party(root: &Path, directory: &Path) -> bool {
    directory == root
        || directory.starts_with(root.join("crates"))
        || directory.starts_with(root.join("xtask"))
}

fn sources(root: &Path, directory: &Path) -> anyhow::Result<Vec<Source>> {
    let mut sources = Vec::new();
    for (name, test_directory) in [
        ("src", false),
        ("tests", true),
        ("examples", true),
        ("benches", true),
    ] {
        let base = directory.join(name);
        if !base.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        crate::source_files(&base, &mut files)?;
        for file in files {
            let text = fs::read_to_string(&file)?;
            let relative = file.strip_prefix(&base).unwrap_or(&file);
            let test_file = test_directory || is_test_module(relative);
            let facts = facts::collect(&text, test_file)
                .with_context(|| format!("parse {}", file.display()))?;
            sources.push(Source {
                path: file
                    .strip_prefix(root)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/"),
                module: module_name(relative, test_directory),
                test_file,
                lines: text.lines().count(),
                complexity_expects: crate::lint_policy::inspect(&text)?.0,
                facts,
            });
        }
    }
    Ok(sources)
}

fn is_test_module(relative: &Path) -> bool {
    relative
        .components()
        .any(|component| matches!(component.as_os_str().to_str(), Some("tests" | "tests.rs")))
}

fn module_name(relative: &Path, test_directory: bool) -> String {
    if test_directory {
        return "(tests)".into();
    }
    let first = relative
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .unwrap_or_default();
    match first.strip_suffix(".rs").unwrap_or(first) {
        "lib" | "main" => "(root)".into(),
        module => module.to_owned(),
    }
}

fn type_matrix(owner: &str, sources: &[Source]) -> anyhow::Result<matrix::Matrix> {
    let definitions: Vec<_> = sources
        .iter()
        .flat_map(|source| {
            source
                .facts
                .structs
                .iter()
                .filter(|definition| definition.name == owner)
                .map(move |definition| (source.path.as_str(), definition))
        })
        .collect();
    let [(_, definition)] = definitions.as_slice() else {
        if definitions.is_empty() {
            bail!("no production struct named {owner} in this package");
        }
        let files: Vec<_> = definitions.iter().map(|(path, _)| *path).collect();
        bail!(
            "{owner} is ambiguous: defined in {}; the survey keys types by name",
            files.join(", ")
        );
    };
    let rows = sources
        .iter()
        .flat_map(|source| {
            source
                .facts
                .impls
                .iter()
                .filter(|implementation| implementation.owner == owner)
                .flat_map(move |implementation| {
                    implementation
                        .methods
                        .iter()
                        .map(move |method| matrix::Row {
                            name: method.name.clone(),
                            file: source.path.clone(),
                            lines: method.lines,
                            touches: method.touches.iter().cloned().collect(),
                        })
                })
        })
        .collect();
    Ok(matrix::build(owner, &definition.fields, rows))
}
