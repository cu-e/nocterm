//! Fixtures describe the isolation rules, never today's workspace.
use std::{collections::BTreeMap, path::PathBuf};

use serde_json::json;

use super::{
    gpui,
    graph::{self, Graph},
    normal,
    policy::Policy,
    source,
};
use crate::ratchet::{self, Module, Symbols};

const POLICY: &str = r#"
[isolation]
gpui = "gpui-pre"
app = "nocterm"

[isolation.external]
base = ["serde", "serde_core", "serde_derive", "thiserror", "thiserror-impl"]
extra = ["toml"]

[isolation.reviewed]
toml = "Parsed by the adapter only."

[isolation.crates.nocterm-plugin-model]
kind = "no-std"
closure = []
direct = ["serde", "thiserror"]
external = ["base"]

[isolation.crates.nocterm-plugin-api]
kind = "no-std"
closure = ["nocterm-plugin-model"]
direct = ["serde", "thiserror"]
external = ["base"]

[isolation.crates.nocterm-plugin-packages]
kind = "std"
closure = ["nocterm-plugin-api", "nocterm-plugin-model", "nocterm-core"]
direct = ["serde", "thiserror", "toml"]
external = ["base", "extra"]

[isolation.crates.nocterm-plugin-ports]
kind = "gpui"
closure = ["nocterm-plugin-api", "nocterm-plugin-model"]
direct = ["gpui-pre", "thiserror"]
external = ["base"]

[isolation.crates.nocterm-plugin-host]
kind = "gpui"
closure = ["nocterm-plugin-ports", "nocterm-plugin-api", "nocterm-plugin-model"]
direct = ["gpui-pre", "thiserror"]
external = ["base"]

[isolation.crates.nocterm-plugin-view]
kind = "gpui"
closure = ["nocterm-plugin-api", "nocterm-plugin-model"]
direct = ["gpui-pre"]
external = ["base"]

[isolation.io_exceptions]
nocterm_plugin_host_launch = "Fixture exception for a helper-process launch module."
nocterm_plugin_packages = "Fixture exception for the file adapter."

[isolation.gpui_allowed]
notify = "Fixture: repaint request."

[isolation.providers]
allowed = ["nocterm", "nocterm-plugin-host", "nocterm-ui"]

[isolation.compile_out]
crates = ["nocterm-plugin-api", "nocterm-plugin-packages"]
features = { nocterm-plugins-ui = "user" }
test_features = { nocterm-plugin-api = "test-support" }
banned_features = { wasmi = "auto-dispatch" }

[isolation.identifiers.nocterm-plugins-ui]
forbidden = ["SettingsHandle", "settings"]
"#;

fn policy() -> Policy {
    Policy::parse(POLICY).unwrap()
}

pub(super) struct Package {
    name: &'static str,
    member: bool,
    /// `(dependency, kind)` where kind is "" (normal), "build" or "dev".
    dependencies: Vec<(&'static str, &'static str)>,
    targets: Vec<&'static str>,
    features: Vec<&'static str>,
}

pub(super) fn package(
    name: &'static str,
    dependencies: &[(&'static str, &'static str)],
) -> Package {
    Package {
        name,
        member: name.starts_with("nocterm"),
        dependencies: dependencies.to_vec(),
        targets: vec!["lib"],
        features: Vec::new(),
    }
}

pub(super) fn graph(packages: Vec<Package>) -> Graph {
    let id = |name: &str| format!("{name} 0.0.0 (path+file:///fixture/{name})");
    let described: Vec<_> = packages
        .iter()
        .map(|package| {
            json!({
                "id": id(package.name),
                "name": package.name,
                "manifest_path": format!("/fixture/{}/Cargo.toml", package.name),
                "targets": package.targets.iter().map(|kind| json!({"kind": [kind]})).collect::<Vec<_>>(),
                "dependencies": package.dependencies.iter().map(|(name, kind)| {
                    json!({"name": name, "kind": if kind.is_empty() { None } else { Some(*kind) }})
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let nodes: Vec<_> = packages
        .iter()
        .map(|package| {
            json!({
                "id": id(package.name),
                "features": package.features,
                "deps": package.dependencies.iter().map(|(name, kind)| {
                    json!({"pkg": id(name), "dep_kinds": [{"kind": if kind.is_empty() { None } else { Some(*kind) }}]})
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let members: Vec<_> = packages
        .iter()
        .filter(|package| package.member)
        .map(|package| id(package.name))
        .collect();
    let document = json!({
        "packages": described,
        "workspace_members": members,
        "resolve": {"nodes": nodes},
    });
    Graph::parse(&serde_json::to_vec(&document).unwrap()).unwrap()
}

fn base() -> Vec<Package> {
    vec![
        package("serde", &[]),
        package("thiserror", &[]),
        package("toml", &[]),
        package("gpui-pre", &[("serde", "")]),
        package("nocterm-plugin-model", &[("serde", ""), ("thiserror", "")]),
        package(
            "nocterm-plugin-api",
            &[
                ("nocterm-plugin-model", ""),
                ("serde", ""),
                ("thiserror", ""),
            ],
        ),
    ]
}

fn with(mut packages: Vec<Package>, extra: Package) -> Vec<Package> {
    packages.push(extra);
    packages
}

fn closure(packages: Vec<Package>) -> Vec<String> {
    graph::closure_problems(&policy(), &graph(packages))
        .into_iter()
        .collect()
}

#[test]
fn api_depending_only_on_the_model_passes() {
    assert_eq!(closure(base()), Vec::<String>::new());
}

#[test]
fn api_reaching_core_or_a_new_transitive_crate_fails() {
    let mut packages = base();
    packages.push(package("nocterm-core", &[]));
    packages[5].dependencies.push(("nocterm-core", ""));
    assert!(
        closure(packages)
            .iter()
            .any(|p| p.contains("reaches internal crate nocterm-core"))
    );

    let mut packages = with(base(), package("left-pad", &[]));
    packages[1].dependencies.push(("left-pad", ""));
    let problems = closure(packages);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("unlisted external crate left-pad")),
        "{problems:?}"
    );
}

#[test]
fn dev_dependencies_are_not_part_of_the_closure() {
    let mut packages = with(base(), package("proptest", &[]));
    packages[5].dependencies.push(("proptest", "dev"));
    assert_eq!(closure(packages), Vec::<String>::new());
}

#[test]
fn a_new_direct_adapter_dependency_fails() {
    let mut packages = with(base(), package("reqwest", &[]));
    packages.push(package(
        "nocterm-plugin-packages",
        &[
            ("nocterm-plugin-api", ""),
            ("nocterm-plugin-model", ""),
            ("serde", ""),
            ("thiserror", ""),
            ("reqwest", ""),
        ],
    ));
    packages.push(package("nocterm-core", &[]));
    let problems = closure(packages);
    assert!(
        problems
            .iter()
            .any(|p| p
                .contains("nocterm-plugin-packages depends directly on unlisted crate reqwest")),
        "{problems:?}"
    );
}

#[test]
fn gpui_rows_may_also_reach_the_gpui_closure() {
    let mut packages = with(base(), package("slotmap", &[]));
    packages[3].dependencies.push(("slotmap", ""));
    packages.push(package(
        "nocterm-plugin-view",
        &[
            ("gpui-pre", ""),
            ("nocterm-plugin-api", ""),
            ("nocterm-plugin-model", ""),
        ],
    ));
    assert_eq!(
        closure(packages.iter().map(clone).collect()),
        Vec::<String>::new()
    );
    // The same crate reached by a non-gpui row stays unlisted.
    packages[4].dependencies.push(("slotmap", ""));
    assert!(closure(packages).iter().any(|p| p.contains("slotmap")));
}

fn clone(package: &Package) -> Package {
    Package {
        name: package.name,
        member: package.member,
        dependencies: package.dependencies.clone(),
        targets: package.targets.clone(),
        features: package.features.clone(),
    }
}

#[test]
fn build_scripts_and_foreign_proc_macros_fail() {
    let mut packages = base();
    packages[4].targets.push("custom-build");
    assert!(
        closure(packages)
            .iter()
            .any(|p| p.contains("has a build script"))
    );

    // A proc-macro crate fails even when the row lists it, unless it is an approved derive.
    let listed = Policy::parse(&POLICY.replacen(
        "direct = [\"serde\", \"thiserror\"]",
        "direct = [\"serde\", \"thiserror\", \"evil-macros\", \"serde_derive\"]",
        1,
    ))
    .unwrap();
    let mut macros = package("evil-macros", &[]);
    macros.targets = vec!["proc-macro"];
    let mut derive = package("serde_derive", &[]);
    derive.targets = vec!["proc-macro"];
    let mut packages = with(with(base(), macros), derive);
    packages[4].dependencies.push(("evil-macros", ""));
    packages[4].dependencies.push(("serde_derive", ""));
    let problems = graph::closure_problems(&listed, &graph(packages));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("proc-macro crate evil-macros")),
        "{problems:?}"
    );
    assert!(
        !problems.iter().any(|p| p.contains("serde_derive")),
        "{problems:?}"
    );
}

#[test]
fn only_listed_crates_may_depend_on_the_ports() {
    let mut packages = base();
    packages.push(package("nocterm-plugin-ports", &[]));
    packages.push(package("nocterm-ui", &[("nocterm-plugin-ports", "")]));
    assert!(graph::provider_problems(&policy(), &graph(packages)).is_empty());

    let mut packages = base();
    packages.push(package("nocterm-plugin-ports", &[]));
    packages.push(package("nocterm-files", &[("nocterm-plugin-ports", "")]));
    packages.push(package(
        "nocterm-snippets",
        &[("nocterm-plugin-ports", "dev")],
    ));
    let problems = graph::provider_problems(&policy(), &graph(packages));
    assert_eq!(problems.len(), 1);
    assert!(problems.iter().next().unwrap().contains("nocterm-files"));
}

#[test]
fn the_default_build_cannot_reach_user_only_crates_or_features() {
    let mut app = package("nocterm", &[("nocterm-ui", "")]);
    app.member = true;
    let mut packages = with(base(), app);
    packages.push(package("nocterm-ui", &[]));
    assert!(graph::compile_out_problems(&policy(), &graph(packages)).is_empty());

    let mut app = package("nocterm", &[("nocterm-ui", "")]);
    app.member = true;
    let mut packages = with(base(), app);
    packages.push(package("nocterm-ui", &[("nocterm-plugin-api", "")]));
    let problems = graph::compile_out_problems(&policy(), &graph(packages));
    assert!(
        problems
            .iter()
            .next()
            .unwrap()
            .contains("reaches nocterm-plugin-api")
    );

    let mut ui = package("nocterm-plugins-ui", &[]);
    ui.features = vec!["user"];
    let problems = graph::compile_out_problems(&policy(), &graph(with(base(), ui)));
    assert!(problems.iter().next().unwrap().contains("feature user"));
}

#[test]
fn a_test_only_feature_in_the_normal_closure_is_refused() {
    let tree = "\
nocterm v0.1.0 (/work/nocterm)|user-plugins
nocterm-plugin-api v0.0.0 (/work/nocterm/crates/plugin-api)|
nocterm-plugin-host v0.0.0 (/work/nocterm/crates/plugin-host)|test-support
";
    assert!(normal::test_feature_problems(&policy(), tree).is_empty());

    let tree = tree.replace("plugin-api)|", "plugin-api)|test-support");
    let problems = normal::test_feature_problems(&policy(), &tree);
    assert!(
        problems
            .iter()
            .next()
            .unwrap()
            .contains("nocterm-plugin-api enables test-only feature test-support"),
        "{problems:?}"
    );
}

#[test]
fn a_banned_feature_anywhere_in_the_workspace_is_refused() {
    let tree = "\
nocterm-plugin-wasm v0.0.0 (/work/nocterm/crates/plugin-wasm)|
wasmi v2.0.0|portable-dispatch,std,validate
wasmi_core v2.0.0|std
";
    assert!(normal::banned_feature_problems(&policy(), tree).is_empty());

    let tree = tree.replace(
        "portable-dispatch,std",
        "auto-dispatch,portable-dispatch,std",
    );
    let problems = normal::banned_feature_problems(&policy(), &tree);
    assert!(
        problems
            .iter()
            .next()
            .unwrap()
            .contains("wasmi resolves with the banned feature auto-dispatch"),
        "{problems:?}"
    );
}

#[test]
fn policy_rejects_unreasoned_exceptions_and_unknown_lists() {
    let broken = POLICY.replace("Fixture: repaint request.", " ");
    assert!(Policy::parse(&broken).is_err());
    let broken = POLICY.replace(
        "external = [\"base\"]\n\n[isolation.crates.nocterm-plugin-api]",
        "external = [\"missing\"]\n\n[isolation.crates.nocterm-plugin-api]",
    );
    assert!(Policy::parse(&broken).is_err());
    let broken = POLICY.replace(
        "toml = \"Parsed by the adapter only.\"",
        "stale = \"Listed nowhere.\"",
    );
    assert!(Policy::parse(&broken).is_err());
}

fn module(package: &str, name: &str, text: &str) -> Module {
    let parsed = syn::parse_file(text).unwrap();
    let items = parsed
        .items
        .into_iter()
        .filter(|item| !ratchet::test_only(ratchet::attributes(item)))
        .collect::<Vec<_>>();
    // The loader maps every dependency name to its external crate.
    let mut imports = ratchet::imports(&items);
    imports.insert(
        "tempfile".into(),
        vec!["@external".into(), "tempfile".into()],
    );
    Module {
        name: name.into(),
        package: package.into(),
        layer: "service".into(),
        file: PathBuf::from("lib.rs"),
        imports,
        items,
        io_crates: BTreeMap::from([("tempfile".into(), "tempfile".into())]),
        inner_attributes: parsed.attrs,
    }
}

fn violations(modules: &[Module]) -> Vec<String> {
    let methods = gpui::methods_of(
        "impl Window { pub fn intercept_keystrokes(&mut self) {} pub fn notify(&self) {} pub fn new() {} }
         impl<T> Context<T> { pub fn on_action(&mut self) {} }
         impl App { fn private(&self) {} }
         pub trait AppContext { fn update_window(&mut self); fn new_entity(); }
         impl AppContext for App { fn update_window(&mut self) {} fn new_entity() {} }
         pub trait Unrelated { fn mutate(&mut self); }
         impl Clone for Window { fn clone(&self) -> Self { todo!() } }",
    )
    .unwrap();
    source::problems(&policy(), modules, &Symbols::new(modules), &methods)
        .unwrap()
        .into_iter()
        .collect()
}

fn one(package: &str, text: &str) -> Vec<String> {
    violations(&[module(package, &package.replace('-', "_"), text)])
}

#[test]
fn std_is_unreachable_in_no_std_crates() {
    assert!(
        one(
            "nocterm-plugin-api",
            "#![no_std] extern crate alloc; pub fn f() {}"
        )
        .is_empty()
    );
    let problems = one("nocterm-plugin-api", "pub fn f() {}");
    assert!(
        problems[0].contains("must declare #![no_std]"),
        "{problems:?}"
    );
    let problems = one(
        "nocterm-plugin-api",
        "#![no_std] pub fn f() -> Option<u8> { let _ = std::fs::read(\"x\"); None }",
    );
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("I2:") && p.contains("std::fs::read")),
        "{problems:?}"
    );
    // The implicit prelude is not a use of std.
    assert!(
        one(
            "nocterm-plugin-api",
            "#![no_std] pub fn f() -> Result<Option<u8>, ()> { Ok(None) }"
        )
        .is_empty()
    );
}

#[test]
fn io_paths_are_denied_through_aliases_and_reexports() {
    let problems = one(
        "nocterm-plugin-host",
        "use std::fs as f; pub fn read(p: &str) { let _ = f::read(p); }",
    );
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("I2b:") && p.contains("std::fs::read")),
        "{problems:?}"
    );

    let problems = one(
        "nocterm-plugin-view",
        "pub fn read() { let _ = std::env::var(\"HOME\"); }",
    );
    assert!(
        problems.iter().any(|p| p.contains("std::env::var")),
        "{problems:?}"
    );

    let problems = violations(&[
        module(
            "nocterm-plugin-ports",
            "nocterm_plugin_ports",
            "pub mod launch {} pub fn run() { let _ = launch::Spawn::new(\"sh\"); }",
        ),
        module(
            "nocterm-plugin-ports",
            "nocterm_plugin_ports::launch",
            "pub use std::process::Command as Spawn;",
        ),
    ]);
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("I2b:") && p.contains("std::process")),
        "{problems:?}"
    );

    let problems = one(
        "nocterm-plugin-host",
        "pub fn f() { let _ = tempfile::tempfile(); }",
    );
    assert!(
        problems.iter().any(|p| p.contains("tempfile")),
        "{problems:?}"
    );
}

#[test]
fn a_listed_exception_module_may_use_io() {
    let text = "use std::process::Command; pub fn run() { let _ = Command::new(\"bwrap\"); }";
    let modules = [
        module(
            "nocterm-plugin-host",
            "nocterm_plugin_host",
            "pub fn quiet() {}",
        ),
        module("nocterm-plugin-host", "nocterm_plugin_host_launch", text),
    ];
    assert!(violations(&modules).is_empty());
    let modules = [module(
        "nocterm-plugin-host",
        "nocterm_plugin_host::other",
        text,
    )];
    assert!(!violations(&modules).is_empty());
    let packages = module(
        "nocterm-plugin-packages",
        "nocterm_plugin_packages::store",
        "pub fn f() { let _ = std::fs::remove_file(\"x\"); }",
    );
    assert!(violations(&[packages]).is_empty());
}

#[test]
fn gpui_receiver_methods_are_denied_until_allowed() {
    let problems = one(
        "nocterm-plugin-view",
        "use gpui::Window as W; pub fn grab(window: &mut W) { window.intercept_keystrokes(); }",
    );
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("I3:") && p.contains("intercept_keystrokes")),
        "{problems:?}"
    );

    let problems = one(
        "nocterm-plugin-host",
        "pub fn f(cx: &mut Cx) { cx.on_action(); }",
    );
    assert!(
        problems.iter().any(|p| p.contains("on_action")),
        "{problems:?}"
    );

    // Inside a macro, a listed method passes and an unlisted one fails.
    let problems = one(
        "nocterm-plugin-view",
        "pub fn f(w: &W) { run!(w.notify()); run!(w.on_action()); }",
    );
    assert_eq!(problems.len(), 1, "{problems:?}");

    // A method that comes through a trait the receivers implement is a receiver method too.
    let problems = one(
        "nocterm-plugin-host",
        "pub fn f(cx: &mut Cx) { cx.update_window(); }",
    );
    assert!(
        problems.iter().any(|p| p.contains("update_window")),
        "{problems:?}"
    );

    // Names that are not receiver methods are free, and private ones never counted. Neither
    // are associated functions of the trait, a trait the receivers do not implement, or the
    // methods of a trait that the source does not define.
    assert!(
        one(
            "nocterm-plugin-view",
            "pub fn f() { new(); private(); new_entity(); mutate(); clone(); }"
        )
        .is_empty()
    );
    // The rule applies to gpui rows only.
    assert!(
        one(
            "nocterm-plugin-api",
            "#![no_std] pub fn f(w: &W) { w.on_action(); }"
        )
        .is_empty()
    );
}

#[test]
fn the_vendored_gpui_receivers_are_enumerated() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vendor/gpui-pre/src");
    let names = gpui::receiver_methods(&root).unwrap();
    for name in [
        "intercept_keystrokes",
        "read_from_primary",
        "prompt_for_paths",
        "open_window",
        "set_menus",
        "on_action",
        "screen_capture_sources",
        "read_from_clipboard",
        "update_window",
        "update_entity",
        "read_entity",
        "background_spawn",
    ] {
        assert!(
            names.contains(name),
            "{name} missing from {} names",
            names.len()
        );
    }
    assert!(names.len() > 300, "{} names", names.len());
}

#[test]
fn macros_includes_unsafe_and_path_attributes_are_banned() {
    for (text, needle) in [
        ("macro_rules! m { () => {}; }", "macro_rules!"),
        (
            "pub fn f() { macro_rules! m { () => {}; } }",
            "macro_rules!",
        ),
        (
            "pub static A: &str = include_str!(\"a.txt\");",
            "include_str!",
        ),
        ("#[path = \"../elsewhere.rs\"] mod outside;", "#[path]"),
        ("pub fn f() { core::arch::asm!(\"nop\"); }", "`asm!`"),
        ("core::arch::global_asm!(\"nop\");", "`global_asm!`"),
        ("#[no_mangle] pub fn f() {}", "`#[no_mangle]`"),
        ("#[unsafe(no_mangle)] pub fn f() {}", "`#[no_mangle]`"),
        (
            "#[unsafe(export_name = \"x\")] pub fn f() {}",
            "`#[export_name]`",
        ),
        ("pub fn f() { unsafe { } }", "unsafe"),
        ("pub unsafe fn f() {}", "unsafe fn"),
        (
            "#[derive(Deserialize)] pub struct S { #[serde(with = \"x\")] a: u8 }",
            "`with`",
        ),
        (
            "#[derive(Deserialize)] pub struct S { #[serde(default = \"make\")] a: u8 }",
            "`default`",
        ),
        (
            "#[derive(Deserialize)] pub struct S { #[cfg_attr(feature = \"x\", serde(deserialize_with = \"f\"))] a: u8 }",
            "`deserialize_with`",
        ),
        (
            "#[derive(JsonSchema)] pub struct S { #[schemars(with = \"String\")] a: u8 }",
            "schemars attribute `with`",
        ),
    ] {
        let problems = one("nocterm-plugin-host", text);
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("I4:") && p.contains(needle)),
            "{text}: {problems:?}"
        );
    }
    let allowed = "#[derive(Deserialize)] #[serde(deny_unknown_fields, rename_all = \"kebab-case\")] pub struct S { #[serde(default)] a: u8, #[serde(skip)] b: u8 }";
    assert!(one("nocterm-plugin-host", allowed).is_empty());
}

#[test]
fn extern_crate_is_limited_to_alloc_at_the_root() {
    assert!(
        one(
            "nocterm-plugin-api",
            "#![no_std] extern crate alloc; #[cfg(test)] extern crate std;"
        )
        .is_empty()
    );
    let problems = one(
        "nocterm-plugin-api",
        "#![no_std] extern crate alloc; extern crate std;",
    );
    assert!(
        problems.iter().any(|p| p.contains("`extern crate std`")),
        "{problems:?}"
    );
    let problems = one(
        "nocterm-plugin-api",
        "#![no_std] extern crate alloc as heap;",
    );
    assert!(!problems.is_empty());
    let nested = violations(&[module(
        "nocterm-plugin-api",
        "nocterm_plugin_api::inner",
        "extern crate alloc;",
    )]);
    assert!(!nested.is_empty());
}

#[test]
fn providers_cannot_publicly_rebind_the_ports() {
    for text in [
        "pub use nocterm_plugin_ports::Observe;",
        "pub use nocterm_plugin_ports::*;",
        "pub use nocterm_plugin_ports::{Apply, Observe as Seen};",
        "use nocterm_plugin_ports as ports; pub use ports::Observe;",
        "pub type Source = nocterm_plugin_ports::Observe<u8>;",
    ] {
        let problems = one("nocterm-ui", text);
        assert!(
            problems.iter().any(|p| p.starts_with("I5:")),
            "{text}: {problems:?}"
        );
    }
    assert!(one("nocterm-ui", "use nocterm_plugin_ports::Observe; pub struct S; impl S { pub fn f(_: &dyn Observe<u8>) {} }").is_empty());
    assert!(one("nocterm-plugin-ports", "pub use nocterm_plugin_api::Zone;").is_empty());
}

#[test]
fn the_plugins_page_cannot_name_system_settings_handles() {
    for text in [
        "use nocterm_settings::SettingsHandle; pub fn f(_: SettingsHandle) {}",
        "pub fn f(cx: &mut Cx) { let settings = 1; let _ = settings; }",
        "pub fn f() { show!(settings); }",
    ] {
        let problems = one("nocterm-plugins-ui", text);
        assert!(
            problems.iter().any(|p| p.starts_with("M7:")),
            "{text}: {problems:?}"
        );
    }
    assert!(
        one(
            "nocterm-plugins-ui",
            "pub struct Page; impl Page { pub fn rows(&self) {} }"
        )
        .is_empty()
    );
}
