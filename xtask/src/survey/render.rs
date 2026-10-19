//! Plain-text rendering of a survey for people; agents use `--json`.
use super::{
    matrix::Matrix,
    report::{Survey, Warning},
};
use std::fmt::Write as _;

/// Rows shown per section; the JSON form is complete.
const ROWS: usize = 15;

pub(super) fn text(survey: &Survey) -> String {
    let mut out = String::new();
    header(&mut out, survey);
    section(&mut out, "warnings", survey.warnings.iter().map(warning));
    shapes(&mut out, survey);
    modules(&mut out, survey);
    if let Some(matrix) = &survey.matrix {
        matrix_text(&mut out, matrix);
    }
    out
}

/// One line for `cargo xtask architecture`, or none without warnings.
pub(super) fn warning_summary(survey: &Survey) -> Option<String> {
    let parts: Vec<_> = [
        ("file-lines", "files near the line limit"),
        ("struct-fields", "structs near the field limit"),
        ("impl-files", "types near the impl-spread limit"),
    ]
    .into_iter()
    .filter_map(|(kind, label)| {
        let count = survey
            .warnings
            .iter()
            .filter(|warning| warning.kind == kind)
            .count();
        (count > 0).then(|| format!("{count} {label}"))
    })
    .collect();
    (!parts.is_empty()).then(|| format!("{}: {}", survey.package, parts.join(", ")))
}

fn header(out: &mut String, survey: &Survey) {
    let totals = &survey.totals;
    let _ = writeln!(
        out,
        "{} ({}): {} files, {} lines ({} in test files), {} tests, {} complexity expectations",
        survey.package,
        survey.layer,
        totals.files,
        totals.lines,
        totals.test_file_lines,
        totals.tests,
        totals.complexity_expects,
    );
    if !survey.card_missing.is_empty() {
        let _ = writeln!(out, "card missing: {}", survey.card_missing.join(", "));
    }
}

fn shapes(out: &mut String, survey: &Survey) {
    section(
        out,
        "largest files",
        survey.files.iter().map(|file| {
            format!(
                "{:>5}  {}  tests={} expects={}",
                file.lines, file.path, file.tests, file.complexity_expects
            )
        }),
    );
    section(
        out,
        "types by fields",
        survey.types.iter().map(|row| {
            format!(
                "{:>3} fields ({} visible)  {}  methods={} impl files={}",
                row.fields,
                row.visible_fields,
                row.name,
                row.methods,
                row.impl_files.len()
            )
        }),
    );
}

fn modules(out: &mut String, survey: &Survey) {
    section(
        out,
        "modules",
        survey.modules.iter().map(|module| {
            let references: Vec<_> = module
                .references
                .iter()
                .map(|(target, count)| format!("{target}={count}"))
                .collect();
            format!(
                "{:>6} lines  {}  files={}  refs: {}",
                module.lines,
                module.name,
                module.files,
                references.join(" ")
            )
        }),
    );
    section(
        out,
        "long tests",
        survey.long_tests.iter().map(|test| {
            format!(
                "{:>4} lines  {}:{} {}",
                test.lines, test.file, test.line, test.name
            )
        }),
    );
}

fn matrix_text(out: &mut String, matrix: &Matrix) {
    let _ = writeln!(
        out,
        "\nmatrix {} ({} fields)",
        matrix.owner,
        matrix.fields.len()
    );
    for row in &matrix.methods {
        let _ = writeln!(
            out,
            "  {} [{}, {} lines]: {}",
            row.name,
            row.file,
            row.lines,
            row.touches.join(", ")
        );
    }
    let _ = writeln!(out, "coordinators: {}", matrix.coordinators.join(", "));
    for (index, cluster) in matrix.clusters.iter().enumerate() {
        let _ = writeln!(
            out,
            "candidate owner {}: fields {} | methods {}",
            index + 1,
            cluster.fields.join(", "),
            cluster.methods.join(", ")
        );
    }
    let _ = writeln!(out, "cross-cluster: {}", matrix.cross_cluster.join(", "));
    let _ = writeln!(
        out,
        "no direct fields: {}",
        matrix.no_direct_fields.join(", ")
    );
    let _ = writeln!(
        out,
        "untouched fields: {}",
        matrix.untouched_fields.join(", ")
    );
}

fn warning(warning: &Warning) -> String {
    format!(
        "{}  {}: {} (warn at {}, limit {})",
        warning.kind, warning.subject, warning.value, warning.warn_at, warning.limit
    )
}

fn section(out: &mut String, title: &str, rows: impl Iterator<Item = String>) {
    let rows: Vec<_> = rows.collect();
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n{title} ({}):", rows.len());
    for row in rows.iter().take(ROWS) {
        let _ = writeln!(out, "  {row}");
    }
    if rows.len() > ROWS {
        let _ = writeln!(out, "  … {} more (use --json)", rows.len() - ROWS);
    }
}
