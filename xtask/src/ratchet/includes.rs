//! `include!` is Rust source, not an opaque third-party macro exemption.
use super::source;
use std::collections::BTreeMap;
use syn::{
    Item,
    visit::{self, Visit},
};

pub(super) fn check(items: &[Item]) -> anyhow::Result<()> {
    let mut visitor = Includes {
        imports: source::imports(items),
        found: false,
    };
    for item in items {
        visitor.visit_item(item);
    }
    if visitor.found {
        anyhow::bail!(
            "include! source must use a declared Rust module; included source is not supported by the architecture collector"
        );
    }
    Ok(())
}

struct Includes {
    imports: BTreeMap<String, Vec<String>>,
    found: bool,
}

impl<'ast> Visit<'ast> for Includes {
    fn visit_item(&mut self, item: &'ast Item) {
        if !source::test_only(source::attributes(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let imports = self.imports.clone();
        if let Some((_, items)) = &module.content {
            self.imports = source::imports(items);
        }
        visit::visit_item_mod(self, module);
        self.imports = imports;
    }

    fn visit_field(&mut self, field: &'ast syn::Field) {
        if !source::test_only(&field.attrs) {
            visit::visit_field(self, field);
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

    fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
        if !source::test_only(&expression.attrs) {
            visit::visit_expr_macro(self, expression);
        }
    }

    fn visit_stmt_macro(&mut self, statement: &'ast syn::StmtMacro) {
        if !source::test_only(&statement.attrs) {
            visit::visit_stmt_macro(self, statement);
        }
    }

    fn visit_expr_block(&mut self, expression: &'ast syn::ExprBlock) {
        if !source::test_only(&expression.attrs) {
            visit::visit_expr_block(self, expression);
        }
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if !source::test_only(&local.attrs) {
            visit::visit_local(self, local);
        }
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let imports = self.imports.clone();
        let items: Vec<_> = block
            .stmts
            .iter()
            .filter_map(|statement| {
                if let syn::Stmt::Item(item) = statement {
                    Some(item.clone())
                } else {
                    None
                }
            })
            .collect();
        self.imports.extend(source::imports(&items));
        visit::visit_block(self, block);
        self.imports = imports;
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        let direct = item
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "include");
        let aliased = item
            .path
            .get_ident()
            .and_then(|name| self.imports.get(&name.to_string()))
            .is_some_and(|path| path.last().is_some_and(|name| name == "include"));
        self.found |= direct || aliased;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_dynamic_local_and_aliased_includes_are_rejected() {
        for text in [
            "include!(\"hidden.rs\");",
            "include!(concat!(env!(\"OUT_DIR\"),\"/hidden.rs\"));",
            "fn f(){include!(\"hidden.rs\");}",
            "mod nested{fn f(){use std::include as hidden; hidden!(\"hidden.rs\");}}",
        ] {
            assert!(
                check(&syn::parse_file(text).unwrap().items).is_err(),
                "{text}"
            );
        }
        assert!(
            check(
                &syn::parse_file("#[cfg(test)] mod tests {include!(\"tests.rs\");}")
                    .unwrap()
                    .items
            )
            .is_ok()
        );
    }
    #[test]
    fn test_only_associated_and_variant_includes_are_not_product_source() {
        for text in [
            "fn f(){#[cfg(test)] include!(\"data.rs\");}",
            "fn f(){#[cfg(test)] {include!(\"data.rs\");}}",
            "fn f(){#[cfg(test)] let data = include!(\"data.rs\");}",
            "struct Model; impl Model {#[cfg(test)] const DATA:u8=include!(\"data.rs\");}",
            "trait Model {#[cfg(test)] const DATA:u8=include!(\"data.rs\");}",
            "enum Model {#[cfg(test)] Data(include!(\"data.rs\"))}",
            "struct Model{#[cfg(test)] data:include!(\"data.rs\")}",
        ] {
            assert!(
                check(&syn::parse_file(text).unwrap().items).is_ok(),
                "{text}"
            );
            let production = text.replace("#[cfg(test)]", "");
            assert!(
                check(&syn::parse_file(&production).unwrap().items).is_err(),
                "{production}"
            );
        }
    }
}
