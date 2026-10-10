//! Rejects ambiguous source alternatives before a map can choose one silently.
use super::source::{self, Module};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Item, Meta, Token,
    punctuated::Punctuated,
    visit::{self, Visit},
};

pub(super) fn check(items: &[Item]) -> anyhow::Result<()> {
    let mut visitor = Bindings {
        problems: Vec::new(),
        local_depth: 0,
    };
    visitor.scope(items.iter(), true);
    for item in items {
        visitor.visit_item(item);
    }
    if !visitor.problems.is_empty() {
        anyhow::bail!("source bindings: {}", visitor.problems.join("; "));
    }
    Ok(())
}

pub(super) fn check_modules(modules: &[Module]) -> anyhow::Result<()> {
    let mut groups = BTreeMap::<&str, Vec<&Item>>::new();
    for module in modules {
        check(&module.items)?;
        groups
            .entry(&module.name)
            .or_default()
            .extend(&module.items);
    }
    let mut visitor = Bindings {
        problems: Vec::new(),
        local_depth: 0,
    };
    for items in groups.values() {
        visitor.scope(items.iter().copied(), true);
    }
    if !visitor.problems.is_empty() {
        anyhow::bail!(
            "conditional module bindings: {}",
            visitor.problems.join("; ")
        );
    }
    Ok(())
}

struct Bindings {
    problems: Vec<String>,
    local_depth: usize,
}
impl Bindings {
    fn scope<'a>(&mut self, items: impl Iterator<Item = &'a Item>, module: bool) {
        let mut imports = BTreeMap::new();
        let mut declarations = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        let mut other_declarations = BTreeSet::new();
        let mut conditional_imports = BTreeSet::new();
        let mut modules = BTreeMap::new();
        let mut conditional_structs = BTreeSet::new();
        for item in items.filter(|item| !source::test_only(source::attributes(item))) {
            if matches!(item, Item::Use(_) | Item::ExternCrate(_)) {
                for (name, path) in source::imports(std::slice::from_ref(item)) {
                    if name == "_" {
                        continue;
                    }
                    if conditional(source::attributes(item)) {
                        conditional_imports.insert(name.clone());
                    }
                    if let Some(previous) = imports.insert(name.clone(), path.clone())
                        && previous != path
                    {
                        self.problems.push(format!("ambiguous import {name}: declare one explicit binding instead of competing conditional alternatives"));
                    }
                }
            }
            let declaration = match item {
                Item::Type(item) => {
                    aliases.insert(item.ident.to_string());
                    Some(&item.ident)
                }
                Item::Struct(item) => {
                    if conditional(&item.attrs) {
                        conditional_structs.insert(item.ident.to_string());
                    }
                    Some(&item.ident)
                }
                Item::Enum(item) => Some(&item.ident),
                Item::Trait(item) => Some(&item.ident),
                Item::Mod(item) => {
                    modules.insert(item.ident.to_string(), conditional(&item.attrs));
                    None
                }
                _ => None,
            };
            if let Some(name) = declaration {
                declarations.insert(name.to_string());
                if !matches!(item, Item::Type(_)) {
                    other_declarations.insert(name.to_string());
                }
            }
        }
        for (name, conditioned) in modules {
            if conditioned || conditional_imports.contains(&name) {
                declarations.insert(name.clone());
                other_declarations.insert(name);
            }
        }
        for name in aliases.intersection(&other_declarations) {
            self.problems.push(format!("ambiguous type alias and declaration {name}: conditional implementation owners are unsupported"));
        }
        for name in imports.keys().filter(|name| declarations.contains(*name)) {
            // Platform-specific concrete struct reexports are validated against
            // the complete symbol tree; impl conditions select their owner.
            if module && conditional_structs.contains(name) && conditional_imports.contains(name) {
                continue;
            }
            self.problems.push(format!("ambiguous imported and declared type {name}: conditional alternatives are unsupported"));
        }
    }
}

impl<'ast> Visit<'ast> for Bindings {
    fn visit_item(&mut self, item: &'ast Item) {
        if !source::test_only(source::attributes(item)) {
            visit::visit_item(self, item);
        }
    }
    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attrs = match item {
            syn::ImplItem::Fn(i) => &i.attrs,
            syn::ImplItem::Const(i) => &i.attrs,
            syn::ImplItem::Type(i) => &i.attrs,
            syn::ImplItem::Macro(i) => &i.attrs,
            _ => {
                visit::visit_impl_item(self, item);
                return;
            }
        };
        if !source::test_only(attrs) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let attrs = match item {
            syn::TraitItem::Fn(i) => &i.attrs,
            syn::TraitItem::Const(i) => &i.attrs,
            syn::TraitItem::Type(i) => &i.attrs,
            syn::TraitItem::Macro(i) => &i.attrs,
            _ => {
                visit::visit_trait_item(self, item);
                return;
            }
        };
        if !source::test_only(attrs) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_variant(&mut self, variant: &'ast syn::Variant) {
        if !source::test_only(&variant.attrs) {
            visit::visit_variant(self, variant);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident("path"))
            .count()
            > 1
        {
            self.problems.push(format!(
                "multiple paths on module {} are unsupported",
                item.ident
            ));
        }
        if item
            .attrs
            .iter()
            .any(|attribute| conditional_path(&attribute.meta))
        {
            self.problems.push(format!("conditional path on module {} is unsupported; use separate explicitly named modules", item.ident));
        }
        if let Some((_, items)) = &item.content {
            if item
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("path"))
            {
                self.problems.push(format!(
                    "path on inline module {} is unsupported; declare an external module",
                    item.ident
                ));
            }
            self.scope(items.iter(), self.local_depth == 0);
        }
        visit::visit_item_mod(self, item);
    }
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.scope(
            block.stmts.iter().filter_map(|statement| {
                if let syn::Stmt::Item(item) = statement {
                    Some(item)
                } else {
                    None
                }
            }),
            false,
        );
        self.local_depth += 1;
        visit::visit_block(self, block);
        self.local_depth -= 1;
    }
}

fn conditional(attributes: &[syn::Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
}

fn conditional_path(meta: &Meta) -> bool {
    let Meta::List(list) = meta else { return false };
    if !list.path.is_ident("cfg_attr") {
        return false;
    }
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .is_ok_and(|nested| {
            nested
                .iter()
                .skip(1)
                .any(|meta| meta.path().is_ident("path") || conditional_path(meta))
        })
}
