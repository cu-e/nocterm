//! Platform alternatives retain guarded effects or fail with an explicit diagnostic.
use super::*;
use syn::Item;

#[test]
fn prelude_containers_have_foreign_impl_owners() {
    for (owner, canonical) in [
        ("Vec<u8>", "std::vec::Vec"),
        ("Box<u8>", "std::boxed::Box"),
        ("String", "std::string::String"),
        ("Option<u8>", "std::option::Option"),
        ("Result<(), u8>", "std::result::Result"),
        ("u64", "core::primitive::u64"),
        ("bool", "core::primitive::bool"),
        ("str", "core::primitive::str"),
        ("f64", "core::primitive::f64"),
        ("usize", "core::primitive::usize"),
    ] {
        let text = format!(
            "trait Port {{}} type Owner = {owner}; impl Port for {owner} {{}} impl Port for Owner {{}}"
        );
        let modules = [module("fixture", "lib.rs", &text)];
        let symbols = Symbols::new(&modules);
        assert_eq!(
            symbols.canonical_type("fixture", &syn::parse_quote!(Owner), &modules[0].imports),
            canonical
        );
        assert!(!counts(&modules).metrics.contains_key("impl_spread"));
    }
}

#[test]
fn explicit_local_owners_override_the_implicit_prelude() {
    for (text, expected) in [
        (
            "struct Vec; impl gpui_kit::Global for Vec {}",
            "fixture::Vec",
        ),
        (
            "struct u64; impl gpui_kit::Global for u64 {}",
            "fixture::u64",
        ),
        (
            "struct State; use crate::State as Vec; impl gpui_kit::Global for Vec {}",
            "fixture::State",
        ),
        (
            "use crate::exports::*; impl gpui_kit::Global for Vec {}",
            "fixture::exports::Vec",
        ),
    ] {
        let modules = [
            module("fixture", "lib.rs", text),
            module("fixture::exports", "exports.rs", "pub struct Vec;"),
        ];
        assert_eq!(counts(&modules).metrics["globals"][expected], 1);
    }
}

#[test]
fn history_projection_vector_impls_do_not_become_local_owners() {
    let modules = [module(
        "nocterm_ai::history::projection",
        "projection.rs",
        "trait HistoryPrompts {} struct SavedPrompt; impl HistoryPrompts for Vec<SavedPrompt> {} impl HistoryPrompts for Vec<std::sync::Arc<SavedPrompt>> {}",
    )];
    assert!(!counts(&modules).metrics.contains_key("impl_spread"));
}

#[test]
fn prelude_fallback_never_hides_missing_local_owners() {
    for text in [
        "impl Missing {}",
        "trait Port {} impl Port for crate::Vec<u8> {}",
        "trait Port {} impl Port for crate::u64 {}",
        "trait Port {} impl Port for self::Option<u8> {}",
        "use crate::Missing as Vec; trait Port {} impl Port for Vec<u8> {}",
        "type Owner = Missing; trait Port {} impl Port for Owner {}",
    ] {
        let modules = [module("fixture", "lib.rs", text)];
        let error = metrics::collect(&modules, &Symbols::new(&modules), &BTreeMap::new())
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("unresolved local impl owner"),
            "{text}: {error}"
        );
    }
}

#[test]
fn concrete_platform_struct_reexports_keep_impl_ownership() {
    let modules = [
        module(
            "fixture",
            "lib.rs",
            "#[cfg(unix)] struct Observer; #[cfg(windows)] use windows::Observer; #[cfg(unix)] impl Observer{}",
        ),
        module(
            "fixture::windows",
            "windows.rs",
            "struct Observer; impl Observer{}",
        ),
    ];
    let symbols = Symbols::new(&modules);
    counts(&modules);
    let Item::Impl(implementation) = &modules[0].items[2] else {
        panic!("fixture")
    };
    assert_eq!(
        symbols.implementation_type(
            "fixture",
            implementation,
            &syn::parse_quote!(Observer),
            &modules[0].imports
        ),
        "fixture::Observer"
    );
}

#[test]
fn external_or_ambiguous_struct_alternatives_cannot_hide_effects() {
    for text in [
        "#[cfg(unix)] struct File; #[cfg(windows)] use std::fs::File; fn f(){File::open(\"x\");}",
        "fn f(){#[cfg(unix)] struct File; #[cfg(windows)] use std::fs::File; File::open(\"x\");}",
        "fn f(){mod nested {#[cfg(unix)] struct File; #[cfg(windows)] use std::fs::File; fn read(){File::open(\"x\");}}}",
        "#[cfg(unix)] type Text=String; #[cfg(windows)] struct Text; fn f()->Result<(),Text>{todo!()}",
        "#[cfg(unix)] mod files{} #[cfg(windows)] use std::fs as files;",
        "#[cfg(windows)] use std::fs as files; #[cfg(unix)] mod files{}",
    ] {
        let modules = [module("fixture", "lib.rs", text)];
        assert!(
            metrics::collect(&modules, &Symbols::new(&modules), &BTreeMap::new()).is_err(),
            "{text}"
        );
    }
}

#[test]
fn same_import_alternatives_and_module_value_names_are_unambiguous() {
    let modules = [
        module(
            "fixture",
            "lib.rs",
            "#[cfg(unix)] use std::string::String as Text; #[cfg(windows)] use std::string::String as Text; #[doc=\"module\"] mod client{} use client::client; fn f()->Result<(),Text>{todo!()}",
        ),
        module("fixture::client", "client.rs", "fn client(){}"),
    ];
    assert_eq!(counts(&modules).metrics["string_errors"]["fixture"], 1);
}

#[test]
fn conditional_ufcs_aliases_preserve_global_reads_and_shutdown_hooks() {
    let modules = [module(
        "fixture",
        "lib.rs",
        "#[cfg(unix)] type Ui=gpui_kit::App; #[cfg(windows)] type Ui=crate::Plain; fn f(){Ui::global::<u8>(); Ui::on_app_quit();}",
    )];
    let counts = counts(&modules);
    assert_eq!(counts.metrics["global_reads"]["fixture"], 1);
    assert_eq!(counts.metrics["quit_handlers"]["fixture::f"], 1);
}

#[test]
fn conditional_local_globs_keep_guarded_effects_and_reject_ambiguous_types() {
    let root = module(
        "fixture",
        "lib.rs",
        "#[cfg(unix)] use unix::*; #[cfg(windows)] use windows::*; struct Service; impl G for Service{} fn read_it(){action(\"x\");}",
    );
    let mut root = root;
    root.layer = "domain".into();
    let mut modules = [
        root,
        module(
            "fixture::unix",
            "unix.rs",
            "pub use gpui_kit::Global as G; pub use std::fs::read as action; pub use std::string::String as Text;",
        ),
        module(
            "fixture::windows",
            "windows.rs",
            "pub trait G{} pub fn action(_: &str){} pub type Text=bool;",
        ),
    ];
    let observed = counts(&modules);
    assert_eq!(observed.metrics["globals"]["fixture::Service"], 1);
    assert_eq!(observed.metrics["domain_io"]["fixture::std::fs::read"], 1);
    modules[0].items.push(syn::parse_quote!(
        fn read() -> Result<(), Text> {
            todo!()
        }
    ));
    assert!(metrics::collect(&modules, &Symbols::new(&modules), &BTreeMap::new()).is_err());
}
