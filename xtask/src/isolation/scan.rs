//! One syntactic pass over a module's production code, collecting what the isolation rules read.
use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{TokenStream, TokenTree};
use syn::{
    Attribute, Item, Meta, Token, Visibility,
    punctuated::Punctuated,
    visit::{self, Visit},
};

use crate::ratchet::{self, Module};

/// Serde and schemars keys whose value names code by path or string.
const PATH_KEYS: &[&str] = &[
    "with",
    "serialize_with",
    "deserialize_with",
    "skip_serializing_if",
    "schema_with",
    "from",
    "into",
    "try_from",
    "remote",
    "getter",
    "crate",
];

/// Macros that pull source or data from files, or emit machine code.
const INCLUDES: &[&str] = &[
    "include",
    "include_str",
    "include_bytes",
    "asm",
    "global_asm",
];

/// Attributes that choose a symbol name or section of the linked program.
const SYMBOL_ATTRIBUTES: &[&str] = &["no_mangle", "export_name", "link_section", "link_name"];

#[derive(Default)]
pub(super) struct Facts {
    /// Every identifier token, including those inside macro invocations.
    pub(super) identifiers: BTreeSet<String>,
    /// Violations of the structural bans (I4).
    pub(super) structural: Vec<String>,
    /// `pub use` and `pub type` bindings as `(alias, path)`, for the provider re-export rule.
    pub(super) public_bindings: Vec<(String, Vec<String>)>,
}

/// Scans a module's own items; inline child modules are scanned as modules of their own.
pub(super) fn facts(module: &Module) -> Facts {
    let root = !module.name.contains("::");
    let mut scanner = Scanner {
        facts: Facts::default(),
        root,
    };
    for item in &module.items {
        if let Item::Mod(item) = item {
            // The loader follows `#[path]`, which can leave the crate directory.
            if item.attrs.iter().any(|attr| attr.path().is_ident("path")) {
                scanner.problem(format!("`#[path]` on `mod {}`", item.ident));
            }
            continue;
        }
        scanner.bindings(item);
        scanner.visit_item(item);
    }
    scanner.facts
}

struct Scanner {
    facts: Facts,
    root: bool,
}

impl Scanner {
    fn bindings(&mut self, item: &Item) {
        match item {
            Item::Use(item) if matches!(item.vis, Visibility::Public(_)) => {
                let mut names = BTreeMap::new();
                ratchet::flatten(&item.tree, &[], &mut names);
                self.facts.public_bindings.extend(names);
            }
            Item::Type(item) if matches!(item.vis, Visibility::Public(_)) => {
                if let syn::Type::Path(ty) = item.ty.as_ref() {
                    let path = ty
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect();
                    self.facts
                        .public_bindings
                        .push((item.ident.to_string(), path));
                }
            }
            _ => {}
        }
    }

    fn problem(&mut self, text: impl Into<String>) {
        self.facts.structural.push(text.into());
    }

    fn tokens(&mut self, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Ident(ident) => self.identifier(&ident),
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
        }
    }

    fn identifier(&mut self, ident: &proc_macro2::Ident) {
        let name = ident.to_string();
        self.facts
            .identifiers
            .insert(name.strip_prefix("r#").unwrap_or(&name).to_owned());
    }

    fn meta(&mut self, meta: &Meta) {
        let Some(owner) = meta.path().get_ident().map(ToString::to_string) else {
            return;
        };
        match owner.as_str() {
            name if SYMBOL_ATTRIBUTES.contains(&name) => {
                self.problem(format!("`#[{name}]` attribute"));
            }
            "unsafe" => {
                let Meta::List(list) = meta else { return };
                if let Ok(nested) =
                    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                {
                    for item in &nested {
                        self.meta(item);
                    }
                }
            }
            "serde" | "schemars" => {
                let Meta::List(list) = meta else { return };
                match list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
                    Ok(nested) => {
                        for item in nested {
                            self.path_key(&owner, &item);
                        }
                    }
                    Err(_) => self.problem(format!("unparsable {owner} attribute")),
                }
            }
            "cfg_attr" => {
                let Meta::List(list) = meta else { return };
                if let Ok(nested) =
                    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                {
                    for item in nested.iter().skip(1) {
                        self.meta(item);
                    }
                }
            }
            _ => {}
        }
    }

    fn path_key(&mut self, owner: &str, meta: &Meta) {
        let Some(key) = meta.path().get_ident().map(ToString::to_string) else {
            return;
        };
        let named = matches!(meta, Meta::NameValue(_));
        if PATH_KEYS.contains(&key.as_str()) || (key == "default" && named) {
            self.problem(format!("{owner} attribute `{key}` takes a path"));
        }
    }
}

impl<'ast> Visit<'ast> for Scanner {
    fn visit_item(&mut self, item: &'ast Item) {
        if !ratchet::test_only(ratchet::attributes(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_field(&mut self, field: &'ast syn::Field) {
        if !ratchet::test_only(&field.attrs) {
            visit::visit_field(self, field);
        }
    }

    fn visit_variant(&mut self, variant: &'ast syn::Variant) {
        if !ratchet::test_only(&variant.attrs) {
            visit::visit_variant(self, variant);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attrs = match item {
            syn::ImplItem::Fn(i) => &i.attrs,
            syn::ImplItem::Const(i) => &i.attrs,
            syn::ImplItem::Type(i) => &i.attrs,
            syn::ImplItem::Macro(i) => &i.attrs,
            _ => &[][..],
        };
        if !ratchet::test_only(attrs) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let attrs = match item {
            syn::TraitItem::Fn(i) => &i.attrs,
            syn::TraitItem::Const(i) => &i.attrs,
            syn::TraitItem::Type(i) => &i.attrs,
            syn::TraitItem::Macro(i) => &i.attrs,
            _ => &[][..],
        };
        if !ratchet::test_only(attrs) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        self.identifier(ident);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.meta(&attribute.meta);
        visit::visit_attribute(self, attribute);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        if !(self.root && item.ident == "alloc" && item.rename.is_none()) {
            self.problem(format!("`extern crate {}` is not allowed", item.ident));
        }
        visit::visit_item_extern_crate(self, item);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(last) = mac.path.segments.last() {
            let name = last.ident.to_string();
            if name == "macro_rules" {
                self.problem("`macro_rules!` is not allowed");
            } else if INCLUDES.contains(&name.as_str()) {
                self.problem(format!("`{name}!` is not allowed"));
            }
        }
        self.tokens(mac.tokens.clone());
        visit::visit_macro(self, mac);
    }

    fn visit_expr_unsafe(&mut self, node: &'ast syn::ExprUnsafe) {
        self.problem("`unsafe` block");
        visit::visit_expr_unsafe(self, node);
    }

    fn visit_signature(&mut self, node: &'ast syn::Signature) {
        if node.unsafety.is_some() {
            self.problem(format!("`unsafe fn {}`", node.ident));
        }
        visit::visit_signature(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if node.unsafety.is_some() {
            self.problem("`unsafe impl`");
        }
        visit::visit_item_impl(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        if node.unsafety.is_some() {
            self.problem(format!("`unsafe trait {}`", node.ident));
        }
        visit::visit_item_trait(self, node);
    }

    fn visit_item_foreign_mod(&mut self, node: &'ast syn::ItemForeignMod) {
        self.problem("`extern` block");
        visit::visit_item_foreign_mod(self, node);
    }
}
