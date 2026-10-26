//! The actions reference: every action the application declares, with its default bindings.
//!
//! Actions are declared with `actions!` (plain commands) or `#[derive(Action)]` (commands that
//! carry data, such as `SwitchLayout { id }`). The reference reads both from source, and the
//! keymap check fails on a binding to an action that is not declared.
use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path};

use anyhow::bail;
use proc_macro2::TokenTree;
use serde::Deserialize;

#[derive(Deserialize)]
struct Keymap {
    section: Vec<Section>,
}
#[derive(Deserialize)]
struct Section {
    context: Option<String>,
    #[serde(default)]
    os: Vec<String>,
    bindings: BTreeMap<String, String>,
}

/// The actions a source file declares, by `namespace::Name`, with their doc comments.
fn action_declarations(source: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let mut actions = BTreeMap::new();
    for item in syn::parse_file(source)?.items {
        match item {
            syn::Item::Macro(item) => declared_by_macro(item, &mut actions)?,
            syn::Item::Struct(item) => declared_by_derive(&item, &mut actions)?,
            _ => {}
        }
    }
    Ok(actions)
}

/// The actions of one `actions!(namespace, [ ... ])` invocation.
fn declared_by_macro(
    item: syn::ItemMacro,
    actions: &mut BTreeMap<String, String>,
) -> anyhow::Result<()> {
    if !item
        .mac
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "actions")
    {
        return Ok(());
    }
    let mut tokens = item.mac.tokens.into_iter();
    let Some(TokenTree::Ident(namespace)) = tokens.next() else {
        bail!("action namespace must be an identifier");
    };
    let Some(TokenTree::Punct(_)) = tokens.next() else {
        bail!("expected action namespace separator");
    };
    let Some(TokenTree::Group(group)) = tokens.next() else {
        bail!("expected actions list");
    };
    let mut description = Vec::new();
    for token in group.stream() {
        match token {
            TokenTree::Group(group) if group.delimiter() == proc_macro2::Delimiter::Bracket => {
                let attribute = group.stream().to_string();
                if let Some(literal) = attribute.strip_prefix("doc = ") {
                    description.push(
                        syn::parse_str::<syn::LitStr>(literal)?
                            .value()
                            .trim()
                            .to_owned(),
                    );
                }
            }
            TokenTree::Ident(name) => {
                actions.insert(format!("{namespace}::{name}"), description.join(" "));
                description.clear();
            }
            _ => {}
        }
    }
    Ok(())
}

/// The action of a type that derives `Action` and names its namespace with `#[action(..)]`.
fn declared_by_derive(
    item: &syn::ItemStruct,
    actions: &mut BTreeMap<String, String>,
) -> anyhow::Result<()> {
    let mut derives_action = false;
    for attribute in item
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
    {
        attribute.parse_nested_meta(|meta| {
            derives_action |= meta
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "Action");
            Ok(())
        })?;
    }
    if !derives_action {
        return Ok(());
    }
    let mut namespace = None;
    for attribute in item
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("action"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("namespace") {
                namespace = Some(meta.value()?.parse::<syn::Ident>()?.to_string());
            } else if meta.input.peek(syn::Token![=]) {
                meta.value()?.parse::<syn::Expr>()?;
            }
            Ok(())
        })?;
    }
    let Some(namespace) = namespace else {
        bail!("`{}` derives Action without a namespace", item.ident);
    };
    let description = item
        .attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(meta) if meta.path.is_ident("doc") => match &meta.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                }) => Some(text.value().trim().to_owned()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    actions.insert(format!("{namespace}::{}", item.ident), description);
    Ok(())
}

/// `docs/reference/actions.md`: every declared action with its default bindings.
pub(crate) fn render(root: &Path) -> anyhow::Result<String> {
    let mut files = Vec::new();
    crate::source_files(&root.join("crates"), &mut files)?;
    let mut declarations = BTreeMap::new();
    for file in files {
        for (name, description) in action_declarations(&fs::read_to_string(file)?)? {
            if declarations.insert(name.clone(), description).is_some() {
                bail!("duplicate action {name}");
            }
        }
    }
    let keymap: Keymap = toml::from_str(&fs::read_to_string(root.join("assets/keymap.toml"))?)?;
    let mut bindings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for section in keymap.section {
        for (keys, action) in section.bindings {
            if !declarations.contains_key(&action) {
                bail!("keymap references undeclared action {action}");
            }
            let platform = if section.os.is_empty() {
                "all".into()
            } else {
                section.os.join(", ")
            };
            bindings.entry(action).or_default().push(format!(
                "`{keys}` ({platform}; {})",
                section.context.as_deref().unwrap_or("global")
            ));
        }
    }
    let mut text = format!(
        "{}# Actions and default key bindings\n\nDerived from `actions!` declarations, `#[derive(Action)]` types and `assets/keymap.toml`.\n\n| Action | Description | Bindings |\n| --- | --- | --- |\n",
        crate::GENERATED
    );
    for (name, description) in declarations {
        writeln!(
            text,
            "| `{name}` | {} | {} |",
            crate::escape(&description),
            crate::escape(&bindings.remove(&name).unwrap_or_default().join("; "))
        )?;
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_parser_reads_namespaces_and_doc_attributes() {
        let actions = action_declarations("gpui_kit::actions!(example, [ /// First command.\n First, /// Second command.\n Second, ]);").unwrap();
        assert_eq!(actions.get("example::First").unwrap(), "First command.");
        assert_eq!(actions.get("example::Second").unwrap(), "Second command.");
    }

    #[test]
    fn action_parser_reads_derived_actions_with_their_namespace_and_docs() {
        let source = "/// Switch to a layout.\n/// By id.\n#[derive(Clone, PartialEq, Deserialize, Action)]\n#[action(namespace = example, no_json)]\npub struct Switch { pub id: String }\n\n#[derive(Clone)]\nstruct NotAnAction;";
        let actions = action_declarations(source).unwrap();
        assert_eq!(
            actions.get("example::Switch").unwrap(),
            "Switch to a layout. By id."
        );
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn a_derived_action_without_a_namespace_is_refused() {
        let source = "#[derive(Action)]\nstruct Lost;";
        assert!(action_declarations(source).is_err());
    }
}
