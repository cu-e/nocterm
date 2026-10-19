//! Fixtures describe the observable policy, never today's inventory numbers.
use super::{
    baseline::Counts,
    docs, metrics,
    source::{self, Module},
    symbols::Symbols,
};
use std::{collections::BTreeMap, path::PathBuf};

fn module(name: &str, file: &str, text: &str) -> Module {
    let items = syn::parse_file(text)
        .unwrap()
        .items
        .into_iter()
        .filter(|item| !source::test_only(source::attributes(item)))
        .collect::<Vec<_>>();
    Module {
        name: name.into(),
        package: "fixture".into(),
        layer: "feature".into(),
        file: PathBuf::from(file),
        imports: source::imports(&items),
        items,
        io_crates: BTreeMap::new(),
    }
}

fn counts(modules: &[Module]) -> Counts {
    metrics::collect(modules, &Symbols::new(modules), &BTreeMap::new()).unwrap()
}

#[test]
fn debt_must_shrink_in_both_code_and_inventory() {
    let mut initial = Counts::default();
    initial.set("fields", "T", 18);
    let mut growth = Counts::default();
    growth.set("fields", "T", 19);
    assert!(initial.check(&growth).is_err());
    assert!(growth.no_growth(&initial).is_err());
    let mut shrink = Counts::default();
    shrink.set("fields", "T", 16);
    assert!(initial.check(&shrink).is_err());
    assert!(shrink.no_growth(&initial).is_ok());
    assert!(shrink.check(&shrink).is_ok());
    assert!(initial.check(&Counts::default()).is_err());
    assert!(Counts::default().check(&growth).is_err());
}

#[test]
fn equal_type_names_in_different_modules_are_distinct() {
    let modules = [
        module(
            "fixture::left",
            "left.rs",
            "pub struct Model(pub u8); impl Model {}",
        ),
        module(
            "fixture::right",
            "right.rs",
            "pub struct Model(pub u8, pub u8); impl Model {}",
        ),
    ];
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["pub_fields"]["fixture::left::Model"], 1);
    assert_eq!(metrics.metrics["pub_fields"]["fixture::right::Model"], 2);
    assert!(!metrics.metrics.contains_key("impl_spread"));
}

#[test]
fn imports_and_qualified_paths_resolve_to_the_declaring_type() {
    let modules = [
        module("fixture", "lib.rs", "pub struct Model; pub mod child {}"),
        module(
            "fixture::child",
            "child.rs",
            "use crate::Model as Alias; impl Alias {} impl super::Model {} impl crate::Model {}",
        ),
    ];
    let symbols = Symbols::new(&modules);
    let path = syn::parse_str("Alias").unwrap();
    assert_eq!(
        symbols.resolve("fixture::child", &path, &modules[1].imports),
        "fixture::Model"
    );
    let path = syn::parse_str("self::child::Alias").unwrap();
    assert_eq!(
        symbols.resolve("fixture", &path, &modules[0].imports),
        "fixture::Model"
    );
}

#[test]
fn production_metrics_skip_tests_but_count_tuple_visibility_and_string_aliases() {
    let modules = [module(
        "fixture",
        "lib.rs",
        r#"
        use std::result::Result as Outcome;
        use std::string::String as Message;
        struct Data(pub u8, pub(crate) u8, u8);
        fn typed() -> Outcome<(), Message> { todo!() }
        #[cfg(all(test, unix))] struct TestData(pub u8);
        #[cfg(test)] mod tests { fn error() -> Result<(), String> { todo!() } }
        #[test] fn error() -> Result<(), String> { todo!() }
    "#,
    )];
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["string_errors"]["fixture"], 1);
    assert_eq!(metrics.metrics["pub_fields"].len(), 1);
    assert_eq!(metrics.metrics["pub_fields"]["fixture::Data"], 2);
}

#[test]
fn domain_io_is_counted_through_module_and_function_aliases() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        r#"
        use std::fs as disk;
        use std::process::Command as Process;
        fn read() { disk::read("x"); Process::new("x"); }
        fn nested() { use std::fs::read as load; load("x"); }
        #[cfg(test)] fn ignored() { std::fs::read("x"); }
    "#,
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert_eq!(metrics.metrics["domain_io"]["fixture::std::fs::read"], 2);
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::std::process::Command::new"],
        1
    );
}

#[test]
fn documentation_checks_known_members_and_ignores_examples_and_external_apis() {
    let modules = [module(
        "fixture",
        "lib.rs",
        "struct Model; impl Model { fn open() {} }",
    )];
    let symbols = Symbols::new(&modules);
    assert!(
        docs::violations(
            "`Model::open`, `File::open` and `Placeholder::method`.",
            &symbols
        )
        .is_empty()
    );
    assert_eq!(
        docs::violations("`Model::missing` and `fixture::unknown`.", &symbols).len(),
        1
    );
    assert!(docs::violations("```rust\nModel::missing(); // underway\n```", &symbols).is_empty());
    assert_eq!(docs::violations("Status: approved", &symbols).len(), 1);
}

#[test]
fn generic_error_aliases_and_cross_module_strings_cannot_hide_string_errors() {
    let modules = [
        module("fixture", "lib.rs", "pub mod types {} pub mod consumer {}"),
        module(
            "fixture::types",
            "types.rs",
            "pub type Outcome<T,E> = std::result::Result<T,E>; pub type Message=std::string::String; pub type Text=Message;",
        ),
        module(
            "fixture::consumer",
            "consumer.rs",
            "use super::types::{Outcome,Text as Error}; fn a()->Outcome<(),Error>{todo!()} fn b()->Result<(),crate::types::Text>{todo!()}",
        ),
    ];
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["string_errors"]["fixture"], 2);
}

#[test]
fn type_aliases_do_not_create_new_impl_owners() {
    let mut modules = vec![module(
        "fixture",
        "lib.rs",
        "struct Model; type Alias=Model; impl Model {}",
    )];
    for index in 0..4 {
        modules.push(module(
            &format!("fixture::child{index}"),
            &format!("child{index}.rs"),
            "use crate::Alias; impl Alias {}",
        ));
    }
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["impl_spread"]["fixture::Model"], 5);
    assert_eq!(metrics.metrics["impl_spread"].len(), 1);
}

#[test]
fn unresolved_local_impl_owners_are_errors() {
    let modules = [module("fixture", "lib.rs", "impl Missing {}")];
    assert!(metrics::collect(&modules, &Symbols::new(&modules), &BTreeMap::new()).is_err());
}

#[test]
fn existing_domain_http_dependency_does_not_authorize_more_calls() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        "use http::Client as Client; fn fetch(){Client::new();}",
    );
    fixture.layer = "domain".into();
    fixture
        .imports
        .insert("http".into(), vec!["@external".into(), "http".into()]);
    fixture.io_crates.insert("http".into(), "reqwest".into());
    let metrics = counts(&[fixture]);
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::reqwest::Client::new"],
        1
    );
}

#[test]
fn inventory_rejects_typo_metrics_and_zero_allowances() {
    let mut counts = Counts::default();
    counts.set("globalz", "Model", 1);
    assert!(counts.validate().is_err());
    let mut counts = Counts::default();
    counts.set("globals", "Model", 0);
    assert!(counts.validate().is_err());
}

#[test]
fn cyclic_glob_reexports_terminate_without_merging_unrelated_owners() {
    let modules = [
        module("fixture", "lib.rs", "pub use child::*; pub mod child {}"),
        module(
            "fixture::child",
            "child.rs",
            "pub use super::*; pub struct Present; impl Present {}",
        ),
    ];
    let symbols = Symbols::new(&modules);
    let path = syn::parse_str("Present").unwrap();
    assert_eq!(
        symbols.resolve("fixture", &path, &modules[0].imports),
        "fixture::child::Present"
    );
    let missing = syn::parse_str("Missing").unwrap();
    assert!(!symbols.contains(&symbols.resolve("fixture", &missing, &modules[0].imports)));
}

#[test]
fn plain_data_exemption_is_named_and_does_not_cover_stateful_neighbors() {
    let modules = [module(
        "fixture",
        "lib.rs",
        "struct Config{pub value:u8} struct State{pub value:u8}",
    )];
    let policy = BTreeMap::from([("fixture::Config".into(), "Plain settings value".into())]);
    let counts = metrics::collect(&modules, &Symbols::new(&modules), &policy).unwrap();
    assert!(!counts.metrics["pub_fields"].contains_key("fixture::Config"));
    assert_eq!(counts.metrics["pub_fields"]["fixture::State"], 1);
}

#[test]
fn same_identity_in_conditional_branches_keeps_the_largest_shape() {
    let large = (0..17)
        .map(|i| format!("f{i}:u8"))
        .collect::<Vec<_>>()
        .join(",");
    let small = (0..16)
        .map(|i| format!("f{i}:u8"))
        .collect::<Vec<_>>()
        .join(",");
    let text =
        format!("#[cfg(unix)] struct State{{{large}}} #[cfg(windows)] struct State{{{small}}}");
    let metrics = counts(&[module("fixture", "lib.rs", &text)]);
    assert_eq!(metrics.metrics["struct_fields"]["fixture::State"], 17);
}

#[test]
fn global_trait_aliases_and_ufcs_shutdown_calls_remain_guarded() {
    let modules = [module(
        "fixture",
        "lib.rs",
        r#"
        use gpui_kit::Global as G;
        struct Service;
        type Alias=Service;
        impl G for Alias {}
        fn install(cx:&mut gpui_kit::App) {
            gpui_kit::App::on_app_quit(cx, todo!());
            gpui_kit::App::global::<Service>(cx);
        }
        #[cfg(test)] fn ignored(cx:&mut gpui_kit::App) { cx.on_app_quit(todo!()); }
    "#,
    )];
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["globals"]["fixture::Service"], 1);
    assert_eq!(metrics.metrics["quit_handlers"]["fixture::install"], 1);
    assert_eq!(metrics.metrics["global_reads"]["fixture"], 1);
}

#[test]
fn filesystem_type_alias_does_not_authorize_new_file_open_calls() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        "type File=std::fs::File; fn read(){File::open(\"x\");}",
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::std::fs::File::open"],
        1
    );
}

#[test]
fn function_and_const_local_shapes_and_globals_cannot_escape_inventory() {
    let fields = (0..16)
        .map(|index| format!("field{index}:u8"))
        .collect::<Vec<_>>()
        .join(",");
    for body in [
        format!("fn install(){{struct State{{{fields}}} impl gpui_kit::Global for State {{}}}}"),
        format!(
            "const LOCAL:()={{struct State{{{fields}}} impl gpui_kit::Global for State {{}}}};"
        ),
        format!(
            "fn install(){{mod nested{{pub struct State{{{fields}}} impl gpui_kit::Global for State {{}}}}}}"
        ),
    ] {
        let modules = [module("fixture", "lib.rs", &body)];
        let metrics = counts(&modules);
        assert_eq!(
            metrics.metrics.get("struct_fields").map(BTreeMap::len),
            Some(1),
            "local state must be measured: {body}"
        );
        assert_eq!(
            metrics.metrics.get("globals").map(BTreeMap::len),
            Some(1),
            "local Global impl must be measured: {body}"
        );
    }
}

#[test]
fn parenthesized_string_and_default_error_alias_remain_string_errors() {
    for text in [
        "fn read()->Result<(), (String)>{todo!()}",
        "type Outcome<T,E=String>=Result<T,E>; fn read()->Outcome<()>{todo!()}",
    ] {
        let metrics = counts(&[module("fixture", "lib.rs", text)]);
        assert_eq!(
            metrics
                .metrics
                .get("string_errors")
                .and_then(|m| m.get("fixture")),
            Some(&1),
            "signature cannot hide String: {text}"
        );
    }
}

#[test]
fn local_error_aliases_resolve_inside_function_blocks() {
    let modules = [module(
        "fixture",
        "lib.rs",
        "fn operation(){type Message=String; let _f=||->Result<(),Message>{todo!()};}",
    )];
    let metrics = counts(&modules);
    assert_eq!(
        metrics
            .metrics
            .get("string_errors")
            .and_then(|m| m.get("fixture")),
        Some(&1)
    );
}

#[test]
fn test_only_field_types_do_not_add_production_errors_or_io() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        "struct Model{ value:u8, #[cfg(test)] error:Result<(),String>, #[cfg(test)] file:std::fs::File } enum Event{Current, #[cfg(test)] Error(Result<(),String>), #[cfg(test)] File(std::fs::File)}",
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert!(!metrics.metrics.contains_key("string_errors"));
    assert!(!metrics.metrics.contains_key("domain_io"));
}

#[test]
fn test_only_associated_constants_do_not_add_production_debt() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        "struct Model; impl Model{#[cfg(test)] const ERROR:Result<(),String>=Err(String::new()); #[cfg(test)] const FILE:Option<std::fs::File>=None;} trait Port { #[cfg(test)] fn read()->Result<(),String>; #[cfg(test)] const FILE:Option<std::fs::File>; }",
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert!(!metrics.metrics.contains_key("string_errors"));
    assert!(!metrics.metrics.contains_key("domain_io"));
}

#[test]
fn extern_crate_aliases_do_not_hide_globals_or_domain_io() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        "extern crate gpui_kit as gui; extern crate std as os; struct Service; impl gui::Global for Service{} fn read(){os::fs::read(\"x\");}",
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert_eq!(
        metrics
            .metrics
            .get("globals")
            .and_then(|m| m.get("fixture::Service")),
        Some(&1)
    );
    assert_eq!(
        metrics
            .metrics
            .get("domain_io")
            .and_then(|m| m.get("fixture::std::fs::read")),
        Some(&1)
    );
}

#[test]
fn omitted_default_error_parameter_does_not_hide_a_string_signature() {
    let metrics = counts(&[module(
        "fixture",
        "lib.rs",
        "type Outcome<T,E=String>=Result<T,E>; fn read()->Outcome<()>{todo!()}",
    )]);
    assert_eq!(
        metrics
            .metrics
            .get("string_errors")
            .and_then(|m| m.get("fixture")),
        Some(&1)
    );
}

#[test]
fn shared_file_helpers_still_count_as_domain_io_but_queues_do_not() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        r#"
        use nocterm_core::persist::{save as write, WriteQueue};
        use nocterm_core::fs::AtomicPublication as Publication;
        fn persist() { write(todo!(), todo!()); Publication::stage(todo!()); }
        fn queue() -> WriteQueue<u8> { todo!() }
    "#,
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert_eq!(metrics.metrics["domain_io"].len(), 2);
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::nocterm_core::persist::save"],
        1
    );
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::nocterm_core::fs::AtomicPublication::stage"],
        1
    );
}

#[test]
fn same_named_local_states_keep_separate_identities() {
    let fields = (0..16)
        .map(|n| format!("f{n}:u8"))
        .collect::<Vec<_>>()
        .join(",");
    let text =
        format!("fn install(){{ {{struct State{{{fields}}}}} {{struct State{{{fields}}}}} }}");
    let metrics = counts(&[module("fixture", "lib.rs", &text)]);
    assert_eq!(metrics.metrics["struct_fields"].len(), 2);
}

#[test]
fn generic_and_nested_parenthesized_error_aliases_preserve_string_identity() {
    for text in [
        "type Message<T=String>=T; fn read()->Result<(),Message>{todo!()}",
        "type Message<T>=T; fn read()->Result<(),Message<String>>{todo!()}",
        "type Outcome<T,E>=(Result<T,E>); fn read()->Outcome<(),((String))>{todo!()}",
    ] {
        let metrics = counts(&[module("fixture", "lib.rs", text)]);
        assert_eq!(metrics.metrics["string_errors"]["fixture"], 1, "{text}");
    }
}

#[test]
fn address_values_are_pure_but_connections_and_dns_remain_domain_io() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        r#"
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6,
            AddrParseError, TcpStream, TcpListener, UdpSocket, ToSocketAddrs};
        fn address() { IpAddr::from(Ipv4Addr::LOCALHOST); Ipv6Addr::LOCALHOST;
            let _: Option<AddrParseError> = None;
            SocketAddr::new(todo!(), 80); SocketAddrV4::new(todo!(), 80);
            SocketAddrV6::new(todo!(), 80, 0, 0); }
        fn connect() { TcpStream::connect(todo!()); TcpListener::bind(todo!());
            UdpSocket::bind(todo!()); ToSocketAddrs::to_socket_addrs(todo!()); }
    "#,
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert_eq!(metrics.metrics["domain_io"].len(), 4);
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::std::net::TcpStream::connect"],
        1
    );
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::std::net::ToSocketAddrs::to_socket_addrs"],
        1
    );
}

#[test]
fn external_string_import_keeps_identity_when_substituted_in_another_module() {
    let modules = [
        module("fixture", "lib.rs", "pub mod types {} pub mod consumer {}"),
        module(
            "fixture::types",
            "types.rs",
            "pub type Outcome<T,E>=Result<T,E>;",
        ),
        module(
            "fixture::consumer",
            "consumer.rs",
            "use super::types::Outcome; use std::string::String as Message; fn read()->Outcome<(),Message>{todo!()}",
        ),
    ];
    let metrics = counts(&modules);
    assert_eq!(
        metrics
            .metrics
            .get("string_errors")
            .and_then(|m| m.get("fixture")),
        Some(&1)
    );
}

#[test]
fn parenthesized_impl_owners_still_name_the_underlying_local_type() {
    let modules = (0..5).map(|index| module(
        "fixture", &format!("{index}.rs"),
        "struct State; type Alias=(State); impl Alias{} impl gpui_kit::Global for (State){}",
    )).collect::<Vec<_>>();
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["impl_spread"]["fixture::State"], 5);
    assert!(!metrics.metrics["impl_spread"].contains_key("fixture::Alias"));
    assert_eq!(metrics.metrics["globals"]["fixture::State"], 5);
}

#[test]
fn function_local_imports_resolve_self_and_super_from_the_rust_module() {
    let modules = [
        module("fixture", "lib.rs", "struct Root;"),
        module(
            "fixture::child",
            "child.rs",
            r#"
        struct State;
        fn install() { use self::State as Owner; use super::Root as Parent;
            impl gpui_kit::Global for Owner {} impl gpui_kit::Global for Parent {} }
    "#,
        ),
    ];
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["globals"]["fixture::Root"], 1);
    assert_eq!(metrics.metrics["globals"]["fixture::child::State"], 1);
}

#[test]
fn temporary_file_and_directory_adapters_are_domain_io_even_through_aliases() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        r#"
        use tempfile::{NamedTempFile as Temp, tempdir as directory};
        fn write() { Temp::new_in(todo!()); directory(); tempfile::tempfile(); }
        #[cfg(test)] fn ignored() { tempfile::tempdir(); }
    "#,
    );
    fixture.layer = "domain".into();
    fixture
        .io_crates
        .insert("tempfile".into(), "tempfile".into());
    fixture.imports.insert(
        "tempfile".into(),
        vec!["@external".into(), "tempfile".into()],
    );
    let metrics = counts(&[fixture]);
    assert_eq!(metrics.metrics["domain_io"].len(), 3);
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::tempfile::NamedTempFile::new_in"],
        1
    );
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::tempfile::tempdir"],
        1
    );
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::tempfile::tempfile"],
        1
    );
}

#[test]
fn external_globs_for_filesystem_and_gpui_require_explicit_bindings() {
    for text in [
        "use std::fs::*; fn read_it(){read(\"x\");}",
        "use std::fs as files; use files::*; fn read_it(){read(\"x\");}",
        "use gpui_kit::*; struct Service; impl Global for Service{}",
        "use gpui_kit as gui; use gui::*; struct Service; impl Global for Service{}",
        "fn f(){use std::fs::*; read(\"x\");}",
        "fn f(){mod nested{use gpui_kit::*; struct Service; impl Global for Service{}}}",
    ] {
        let modules = [module("fixture", "lib.rs", text)];
        let error = metrics::collect(&modules, &Symbols::new(&modules), &BTreeMap::new())
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("external glob"),
            "{text}: {error}"
        );
    }
}

#[test]
fn local_globs_preserve_external_io_global_and_string_reexports() {
    let mut root = module(
        "fixture",
        "lib.rs",
        r#"
        use exports as exported; use exported::*; struct Service; impl G for Service{}
        fn read_it()->Result<(),Text>{read("x"); todo!()}
    "#,
    );
    root.layer = "domain".into();
    let modules = [
        root,
        module(
            "fixture::exports",
            "exports.rs",
            r#"
        pub use std::fs::read;
        pub use std::string::String as Text;
        pub use gpui_kit::Global as G;
    "#,
        ),
    ];
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["globals"]["fixture::Service"], 1);
    assert_eq!(metrics.metrics["string_errors"]["fixture"], 1);
    assert_eq!(metrics.metrics["domain_io"]["fixture::std::fs::read"], 1);
}

#[test]
fn conditional_competing_imports_cannot_select_the_last_platform() {
    for text in [
        "#[cfg(unix)] use std::fs::read as action; #[cfg(windows)] use crate::pure::action; fn f(){action(\"x\");}",
        "#[cfg(unix)] use gpui_kit::Global as G; #[cfg(windows)] use crate::Pure as G; struct Service; impl G for Service{}",
        "#[cfg(unix)] use std::string::String as Text; #[cfg(windows)] use crate::Plain as Text; fn f()->Result<(),Text>{todo!()}",
        "fn f(){#[cfg(unix)] use std::fs::read as action; #[cfg(windows)] use crate::pure::action; action(\"x\");}",
        "#[cfg(unix)] extern crate std as os; #[cfg(windows)] extern crate pure as os; fn f(){os::fs::read(\"x\");}",
    ] {
        let modules = [module("fixture", "lib.rs", text)];
        let error = metrics::collect(&modules, &Symbols::new(&modules), &BTreeMap::new())
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("ambiguous import"),
            "{text}: {error}"
        );
    }
}

#[test]
fn conditional_type_aliases_preserve_every_error_and_io_alternative() {
    let mut fixture = module(
        "fixture",
        "lib.rs",
        r#"
        #[cfg(unix)] type Text=String; #[cfg(windows)] type Text=bool;
        #[cfg(unix)] type Outcome<T>=Result<T,String>; #[cfg(windows)] type Outcome<T>=Result<T,u8>;
        #[cfg(unix)] type File=std::fs::File; #[cfg(windows)] type File=crate::Plain;
        fn a()->Result<(),Text>{todo!()} fn b()->Outcome<()>{todo!()}
        fn read(){File::open("x");}
    "#,
    );
    fixture.layer = "domain".into();
    let metrics = counts(&[fixture]);
    assert_eq!(metrics.metrics["string_errors"]["fixture"], 3);
    assert_eq!(
        metrics.metrics["domain_io"]["fixture::std::fs::File::open"],
        1
    );
}

#[test]
fn conditional_impl_owner_aliases_fail_closed_instead_of_spreading_types() {
    let modules = [module(
        "fixture",
        "lib.rs",
        r#"
        struct Left; struct Right;
        #[cfg(unix)] type Owner=Left; #[cfg(windows)] type Owner=Right;
        impl Owner{}
    "#,
    )];
    let error = metrics::collect(&modules, &Symbols::new(&modules), &BTreeMap::new())
        .err()
        .unwrap();
    assert!(error.to_string().contains("ambiguous_impl_owner"));
}

#[test]
fn qualified_reexported_module_aliases_preserve_external_ownership() {
    let mut root = module(
        "fixture",
        "lib.rs",
        r#"
        use gateway::*;
        struct Service; impl crate::gui::Global for Service{}
        fn f()->Result<(),crate::text::String>{crate::files::read("x"); todo!()}
    "#,
    );
    root.layer = "domain".into();
    let modules = [
        root,
        module(
            "fixture::gateway",
            "gateway.rs",
            r#"
        pub use std::fs as files; pub use std::string as text; pub use gpui_kit as gui;
    "#,
        ),
    ];
    let metrics = counts(&modules);
    assert_eq!(metrics.metrics["globals"]["fixture::Service"], 1);
    assert_eq!(metrics.metrics["string_errors"]["fixture"], 1);
    assert_eq!(metrics.metrics["domain_io"]["fixture::std::fs::read"], 1);
}

mod conditional;
