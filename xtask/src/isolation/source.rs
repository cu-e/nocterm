//! Source rules over the loaded production modules: `std` reachability (I2, I2b), the gpui
//! receiver allowlist (I3), structural bans (I4), provider re-exports (I5) and the
//! identifier scan (M7).
use std::collections::BTreeSet;

use super::{
    policy::{Kind, Policy},
    scan::{self, Facts},
};
use crate::ratchet::{self, Module, Symbols};

/// The ports crate, whose items only its listed dependents may bind.
const PORTS: &str = "nocterm_plugin_ports";

/// True when some row is bound to the gpui receiver rule and has modules to scan.
pub(super) fn needs_gpui(policy: &Policy, modules: &[Module]) -> bool {
    modules.iter().any(|module| {
        policy
            .crates
            .get(&module.package)
            .is_some_and(|row| row.kind == Kind::Gpui)
    })
}

pub(super) fn problems(
    policy: &Policy,
    modules: &[Module],
    symbols: &Symbols,
    gpui: &BTreeSet<String>,
) -> anyhow::Result<BTreeSet<String>> {
    let mut problems = BTreeSet::new();
    for module in modules {
        let facts = scan::facts(module);
        if let Some(row) = policy.crates.get(&module.package) {
            structural(module, &facts, &mut problems);
            if row.kind == Kind::NoStd && !module.name.contains("::") {
                no_std_root(module, &mut problems);
            }
            if row.kind == Kind::Gpui {
                for name in facts.identifiers.intersection(gpui) {
                    if !policy.gpui_allowed.contains_key(name) {
                        problems.insert(format!(
                            "I3: {} names the gpui receiver method `{name}`",
                            module.name
                        ));
                    }
                }
            }
        }
        if let Some(scan) = policy.identifiers.get(&module.package) {
            for name in &scan.forbidden {
                if facts.identifiers.contains(name) {
                    problems.insert(format!("M7: {} uses the identifier `{name}`", module.name));
                }
            }
        }
        if module.package != "nocterm-plugin-ports" {
            for (alias, path) in &facts.public_bindings {
                if reaches_ports(symbols, module, path) {
                    problems.insert(format!(
                        "I5: {} publicly binds `{alias}` from {PORTS}",
                        module.name
                    ));
                }
            }
        }
    }
    let paths = ratchet::walk_paths(modules, symbols, &|module| {
        policy.crates.contains_key(&module.package)
    })?;
    for (module, name) in paths {
        let Some(module) = modules.iter().find(|candidate| candidate.name == module) else {
            continue;
        };
        let Some(row) = policy.crates.get(&module.package) else {
            continue;
        };
        if row.kind == Kind::NoStd && reaches_std(&name) {
            problems.insert(format!(
                "I2: {} reaches `{name}` in a no_std crate",
                module.name
            ));
        } else if let Some(io) = ratchet::io_path(&name, &module.io_crates, true)
            && !exempt(policy, &module.name)
        {
            problems.insert(format!("I2b: {} reaches the I/O path `{io}`", module.name));
        }
    }
    Ok(problems)
}

/// A path into `std`, other than the names the resolver gives to the implicit prelude.
fn reaches_std(name: &str) -> bool {
    const PRELUDE: &[&str] = &[
        "std::boxed::Box",
        "std::option::Option",
        "std::result::Result",
        "std::string::String",
        "std::vec::Vec",
    ];
    (name == "std" || name.starts_with("std::"))
        && !PRELUDE
            .iter()
            .any(|prelude| name == *prelude || name.starts_with(&format!("{prelude}::")))
}

fn exempt(policy: &Policy, module: &str) -> bool {
    policy
        .io_exceptions
        .keys()
        .any(|key| module == key || module.starts_with(&format!("{key}::")))
}

fn structural(module: &Module, facts: &Facts, problems: &mut BTreeSet<String>) {
    for text in &facts.structural {
        problems.insert(format!("I4: {} has {text}", module.name));
    }
}

/// The crate root of a `no_std` row declares `#![no_std]` and nothing else removes it.
fn no_std_root(module: &Module, problems: &mut BTreeSet<String>) {
    if !module
        .inner_attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("no_std"))
    {
        problems.insert(format!("I2: {} must declare #![no_std]", module.name));
    }
}

fn reaches_ports(symbols: &Symbols, module: &Module, path: &[String]) -> bool {
    let Ok(path) = syn::parse_str::<syn::Path>(&path.join("::")) else {
        return false;
    };
    symbols
        .resolve_candidates(&module.name, &path, &module.imports)
        .iter()
        .any(|name| name == PORTS || name.starts_with(&format!("{PORTS}::")))
}
