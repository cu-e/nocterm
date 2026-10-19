//! Aggregates file facts of one package into a survey with early warnings.
use super::facts::FileFacts;
use serde::Serialize;
use std::collections::BTreeMap;

/// Early-warning thresholds: about four fifths of each enforced limit, so a
/// split is designed before the check fails rather than cut at the cliff.
pub(super) const FILE_WARN_LINES: usize = 640;
pub(super) const FIELD_WARN: usize = 12;
pub(super) const IMPL_FILES_WARN: usize = 4;
pub(super) const TEST_WARN_LINES: usize = 60;

#[derive(Serialize)]
pub(super) struct Survey {
    pub(super) package: String,
    pub(super) layer: String,
    pub(super) totals: Totals,
    pub(super) files: Vec<FileRow>,
    pub(super) types: Vec<TypeRow>,
    pub(super) modules: Vec<ModuleRow>,
    pub(super) long_tests: Vec<TestRow>,
    pub(super) warnings: Vec<Warning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) matrix: Option<super::matrix::Matrix>,
}

#[derive(Default, Serialize)]
pub(super) struct Totals {
    pub(super) files: usize,
    pub(super) lines: usize,
    pub(super) test_file_lines: usize,
    pub(super) tests: usize,
    pub(super) complexity_expects: usize,
}

#[derive(Serialize)]
pub(super) struct FileRow {
    pub(super) path: String,
    pub(super) lines: usize,
    pub(super) tests: usize,
    pub(super) complexity_expects: usize,
}

#[derive(Serialize)]
pub(super) struct TypeRow {
    pub(super) name: String,
    pub(super) definitions: Vec<String>,
    pub(super) fields: usize,
    pub(super) visible_fields: usize,
    pub(super) impl_files: Vec<String>,
    pub(super) methods: usize,
}

#[derive(Serialize)]
pub(super) struct ModuleRow {
    pub(super) name: String,
    pub(super) files: usize,
    pub(super) lines: usize,
    pub(super) references: BTreeMap<String, usize>,
}

#[derive(Serialize)]
pub(super) struct TestRow {
    pub(super) name: String,
    pub(super) file: String,
    pub(super) line: usize,
    pub(super) lines: usize,
}

#[derive(Serialize)]
pub(super) struct Warning {
    pub(super) kind: &'static str,
    pub(super) subject: String,
    pub(super) value: usize,
    pub(super) warn_at: usize,
    pub(super) limit: usize,
}

/// One parsed source file of the package.
pub(super) struct Source {
    pub(super) path: String,
    /// Top-level module (`thread` for `src/thread/queue.rs`), `(root)` or `(tests)`.
    pub(super) module: String,
    pub(super) test_file: bool,
    pub(super) lines: usize,
    pub(super) complexity_expects: usize,
    pub(super) facts: FileFacts,
}

pub(super) fn summarize(package: String, layer: String, sources: &[Source]) -> Survey {
    let mut totals = Totals::default();
    let mut files = Vec::new();
    let mut modules: BTreeMap<String, ModuleRow> = BTreeMap::new();
    let mut long_tests = Vec::new();
    for source in sources {
        totals.files += 1;
        totals.lines += source.lines;
        totals.tests += source.facts.tests.len();
        totals.complexity_expects += source.complexity_expects;
        if source.test_file {
            totals.test_file_lines += source.lines;
        }
        files.push(FileRow {
            path: source.path.clone(),
            lines: source.lines,
            tests: source.facts.tests.len(),
            complexity_expects: source.complexity_expects,
        });
        let module = modules
            .entry(source.module.clone())
            .or_insert_with(|| ModuleRow {
                name: source.module.clone(),
                files: 0,
                lines: 0,
                references: BTreeMap::new(),
            });
        module.files += 1;
        module.lines += source.lines;
        for (target, count) in &source.facts.references {
            if *target != format!("crate::{}", source.module) {
                *module.references.entry(target.clone()).or_default() += count;
            }
        }
        long_tests.extend(
            source
                .facts
                .tests
                .iter()
                .filter(|test| test.lines > TEST_WARN_LINES)
                .map(|test| TestRow {
                    name: test.name.clone(),
                    file: source.path.clone(),
                    line: test.line,
                    lines: test.lines,
                }),
        );
    }
    files.sort_by_key(|file| std::cmp::Reverse(file.lines));
    long_tests.sort_by_key(|test| std::cmp::Reverse(test.lines));
    let types = types(sources);
    let warnings = warnings(&files, &types);
    Survey {
        package,
        layer,
        totals,
        files,
        types,
        modules: modules.into_values().collect(),
        long_tests,
        warnings,
        matrix: None,
    }
}

fn types(sources: &[Source]) -> Vec<TypeRow> {
    let mut types: BTreeMap<&str, TypeRow> = BTreeMap::new();
    for source in sources {
        for definition in &source.facts.structs {
            let row = types
                .entry(&definition.name)
                .or_insert_with(|| empty(&definition.name));
            row.definitions.push(source.path.clone());
            row.fields = row.fields.max(definition.fields.len());
            row.visible_fields = row.visible_fields.max(definition.visible_fields);
        }
    }
    for source in sources {
        for implementation in &source.facts.impls {
            let Some(row) = types.get_mut(implementation.owner.as_str()) else {
                continue;
            };
            if !row.impl_files.contains(&source.path) {
                row.impl_files.push(source.path.clone());
            }
            row.methods += implementation.methods.len();
        }
    }
    let mut types: Vec<_> = types.into_values().collect();
    types.sort_by(|left, right| {
        (right.fields, right.impl_files.len()).cmp(&(left.fields, left.impl_files.len()))
    });
    types
}

fn empty(name: &str) -> TypeRow {
    TypeRow {
        name: name.to_owned(),
        definitions: Vec::new(),
        fields: 0,
        visible_fields: 0,
        impl_files: Vec::new(),
        methods: 0,
    }
}

fn warnings(files: &[FileRow], types: &[TypeRow]) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for file in files.iter().filter(|file| file.lines >= FILE_WARN_LINES) {
        warnings.push(Warning {
            kind: "file-lines",
            subject: file.path.clone(),
            value: file.lines,
            warn_at: FILE_WARN_LINES,
            limit: crate::file_size::MAX_LINES,
        });
    }
    for row in types {
        if row.fields >= FIELD_WARN {
            warnings.push(Warning {
                kind: "struct-fields",
                subject: row.name.clone(),
                value: row.fields,
                warn_at: FIELD_WARN,
                limit: crate::ratchet::MAX_FIELDS,
            });
        }
        if row.impl_files.len() >= IMPL_FILES_WARN {
            warnings.push(Warning {
                kind: "impl-files",
                subject: row.name.clone(),
                value: row.impl_files.len(),
                warn_at: IMPL_FILES_WARN,
                limit: crate::ratchet::MAX_IMPL_FILES,
            });
        }
    }
    warnings
}
