use super::{facts, matrix, module_name, report, type_matrix};
use std::path::Path;

fn source(path: &str, module: &str, text: &str) -> report::Source {
    report::Source {
        path: path.into(),
        module: module.into(),
        test_file: false,
        lines: text.lines().count(),
        complexity_expects: crate::lint_policy::inspect(text).unwrap().0,
        facts: facts::collect(text, false).unwrap(),
    }
}

#[test]
fn production_shapes_exclude_test_code_and_count_visible_fields() {
    let facts = facts::collect(
        "pub struct Model { pub(crate) a: u8, b: u8, #[cfg(test)] probe: u8 }
         struct Pair(pub u8, u8);
         #[cfg(test)] mod tests { struct Fixture { a: u8 } #[test] fn opens() {} #[gpui::test] fn closes() {} }",
        false,
    )
    .unwrap();
    let shapes: Vec<_> = facts
        .structs
        .iter()
        .map(|shape| {
            (
                shape.name.as_str(),
                shape.fields.len(),
                shape.visible_fields,
            )
        })
        .collect();
    assert_eq!(shapes, [("Model", 2, 1), ("Pair", 2, 1)]);
    let tests: Vec<_> = facts.tests.iter().map(|test| test.name.as_str()).collect();
    assert_eq!(tests, ["opens", "closes"]);
}

#[test]
fn methods_record_the_fields_they_reach_through_self() {
    let facts = facts::collect(
        "impl Model {
             fn open(&mut self) { self.queue.push(1); let _ = self.state; }
             fn name() -> &'static str { \"model\" }
             #[cfg(test)] fn probe(&self) -> u8 { self.secret }
         }
         impl Drop for Model { fn drop(&mut self) { self.queue.clear(); } }",
        false,
    )
    .unwrap();
    let methods: Vec<_> = facts
        .impls
        .iter()
        .flat_map(|implementation| &implementation.methods)
        .map(|method| {
            (
                method.name.as_str(),
                method.touches.iter().cloned().collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        methods,
        [
            ("open", vec!["queue".to_owned(), "state".to_owned()]),
            ("name", vec![]),
            ("drop", vec!["queue".to_owned()]),
        ]
    );
}

#[test]
fn production_references_name_workspace_crates_and_sibling_modules() {
    let facts = facts::collect(
        "use nocterm_workspace::{Item, Workspace};
         use crate::panel::AgentPanel;
         use crate::Installed;
         fn open() { nocterm_session::open(); crate::panel::show(); std::mem::drop(()); }
         #[cfg(test)] mod tests { use nocterm_store::Fake; }",
        false,
    )
    .unwrap();
    let references: Vec<_> = facts
        .references
        .iter()
        .map(|(target, count)| (target.as_str(), *count))
        .collect();
    assert_eq!(
        references,
        [
            ("crate::(root)", 1),
            ("crate::panel", 2),
            ("nocterm_session", 1),
            ("nocterm_workspace", 1),
        ]
    );
}

#[test]
fn files_and_types_near_their_limits_are_warned_before_failing() {
    let near = "fn f() {}\n".repeat(report::FILE_WARN_LINES);
    let fields: String = (0..report::FIELD_WARN)
        .map(|index| format!("f{index}: u8,"))
        .collect();
    let sources = [
        source("crates/a/src/near.rs", "near", &near),
        source(
            "crates/a/src/small.rs",
            "small",
            &format!("struct Wide {{ {fields} }}"),
        ),
        source("crates/a/src/one.rs", "one", "impl Wide { fn a(&self) {} }"),
        source("crates/a/src/two.rs", "two", "impl Wide { fn b(&self) {} }"),
        source(
            "crates/a/src/three.rs",
            "three",
            "impl Wide { fn c(&self) {} }",
        ),
        source(
            "crates/a/src/four.rs",
            "four",
            "impl Wide { fn d(&self) {} }",
        ),
    ];
    let survey = report::summarize("a".into(), "domain".into(), &sources);
    let warnings: Vec<_> = survey
        .warnings
        .iter()
        .map(|warning| (warning.kind, warning.subject.as_str()))
        .collect();
    assert_eq!(
        warnings,
        [
            ("file-lines", "crates/a/src/near.rs"),
            ("struct-fields", "Wide"),
            ("impl-files", "Wide"),
        ]
    );
    let summary = super::render::warning_summary(&survey).unwrap();
    assert!(
        summary.starts_with("a: 1 files near the line limit"),
        "{summary}"
    );
}

#[test]
fn fields_changed_together_form_candidate_owners() {
    let row = |name: &str, touches: &[&str]| matrix::Row {
        name: name.into(),
        file: "model.rs".into(),
        lines: 1,
        touches: touches.iter().map(|field| (*field).to_owned()).collect(),
    };
    let fields: Vec<String> = [
        "queue", "pending", "draft", "cursor", "status", "epoch", "title",
    ]
    .map(String::from)
    .into();
    let result = matrix::build(
        "Model",
        &fields,
        vec![
            row("enqueue", &["queue", "pending"]),
            row("flush", &["queue", "pending"]),
            row("edit", &["draft", "cursor"]),
            row("submit", &["draft", "queue"]),
            row("render_title", &[]),
            row(
                "reconnect",
                &["queue", "pending", "draft", "cursor", "status"],
            ),
        ],
    );
    let clusters: Vec<_> = result
        .clusters
        .iter()
        .map(|cluster| (cluster.fields.clone(), cluster.methods.clone()))
        .collect();
    assert_eq!(
        clusters,
        [
            (
                vec!["queue".to_owned(), "pending".to_owned()],
                vec!["enqueue".to_owned(), "flush".to_owned()]
            ),
            (
                vec!["draft".to_owned(), "cursor".to_owned()],
                vec!["edit".to_owned()]
            ),
        ]
    );
    assert_eq!(result.coordinators, ["reconnect"]);
    assert_eq!(result.cross_cluster, ["submit"]);
    assert_eq!(result.no_direct_fields, ["render_title"]);
    assert_eq!(result.coordinator_only_fields, ["status"]);
    assert_eq!(result.untouched_fields, ["epoch", "title"]);
}

#[test]
fn matrices_refuse_types_whose_name_is_ambiguous() {
    let sources = [
        source("a.rs", "a", "struct State { a: u8 }"),
        source("b.rs", "b", "struct State { b: u8 }"),
    ];
    let error = type_matrix("State", &sources).err().unwrap().to_string();
    assert!(error.contains("a.rs, b.rs"), "{error}");
    assert!(type_matrix("Missing", &sources).is_err());
}

#[test]
fn files_belong_to_their_top_level_module() {
    assert_eq!(
        module_name(Path::new("thread/queue/item.rs"), false),
        "thread"
    );
    assert_eq!(module_name(Path::new("thread.rs"), false), "thread");
    assert_eq!(module_name(Path::new("lib.rs"), false), "(root)");
    assert_eq!(module_name(Path::new("gui.rs"), true), "(tests)");
}
