//! Syntactic facts of one source file. Names are not resolved: types are keyed
//! by their last path segment, so same-named types are reported as ambiguous.
use crate::ratchet::test_only;
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Expr, Fields, ImplItem, Member, Type, UseTree, Visibility,
    spanned::Spanned as _,
    visit::{self, Visit},
};

#[derive(Default)]
pub(super) struct FileFacts {
    pub(super) structs: Vec<StructFacts>,
    pub(super) impls: Vec<ImplFacts>,
    pub(super) tests: Vec<TestFacts>,
    /// Production paths into other workspace crates (`nocterm_x`) or this
    /// crate's top-level modules (`crate::x`), counted per occurrence.
    pub(super) references: BTreeMap<String, usize>,
}

pub(super) struct StructFacts {
    pub(super) name: String,
    pub(super) fields: Vec<String>,
    pub(super) visible_fields: usize,
}

pub(super) struct ImplFacts {
    pub(super) owner: String,
    pub(super) methods: Vec<MethodFacts>,
}

pub(super) struct MethodFacts {
    pub(super) name: String,
    pub(super) lines: usize,
    /// Fields reached through `self.<field>`, read or written.
    pub(super) touches: BTreeSet<String>,
}

pub(super) struct TestFacts {
    pub(super) name: String,
    pub(super) line: usize,
    pub(super) lines: usize,
}

/// Collects facts; `test_file` marks files whose every item is test code.
pub(super) fn collect(source: &str, test_file: bool) -> anyhow::Result<FileFacts> {
    let file = syn::parse_file(source)?;
    let mut collector = Collector {
        facts: FileFacts::default(),
        in_test: test_file || test_only(&file.attrs),
    };
    collector.visit_file(&file);
    Ok(collector.facts)
}

struct Collector {
    facts: FileFacts,
    in_test: bool,
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let outer = self.in_test;
        self.in_test |= test_only(&item.attrs);
        visit::visit_item_mod(self, item);
        self.in_test = outer;
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if is_test(&item.attrs) {
            self.facts.tests.push(TestFacts {
                name: item.sig.ident.to_string(),
                line: item.span().start().line,
                lines: lines(item.span()),
            });
            return;
        }
        if test_only(&item.attrs) {
            return;
        }
        visit::visit_item_fn(self, item);
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        if self.in_test || test_only(&item.attrs) {
            return;
        }
        let fields: Vec<_> = production_fields(&item.fields).collect();
        self.facts.structs.push(StructFacts {
            name: item.ident.to_string(),
            visible_fields: fields
                .iter()
                .filter(|(_, field)| !matches!(field.vis, Visibility::Inherited))
                .count(),
            fields: fields.into_iter().map(|(name, _)| name).collect(),
        });
        visit::visit_item_struct(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if self.in_test || test_only(&item.attrs) {
            return;
        }
        let Some(owner) = owner(&item.self_ty) else {
            return visit::visit_item_impl(self, item);
        };
        let methods = item
            .items
            .iter()
            .filter_map(|item| match item {
                ImplItem::Fn(method) if !test_only(&method.attrs) => Some(method),
                _ => None,
            })
            .map(|method| {
                let mut touches = SelfFields::default();
                touches.visit_block(&method.block);
                MethodFacts {
                    name: method.sig.ident.to_string(),
                    lines: lines(method.span()),
                    touches: touches.0,
                }
            })
            .collect();
        self.facts.impls.push(ImplFacts { owner, methods });
        visit::visit_item_impl(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if !self.in_test
            && !test_only(&item.attrs)
            && let Some(target) = use_target(&item.tree)
        {
            *self.facts.references.entry(target).or_default() += 1;
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if !self.in_test {
            let mut segments = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string());
            match (segments.next(), segments.next()) {
                (Some(first), _) if first.starts_with("nocterm_") => {
                    *self.facts.references.entry(first).or_default() += 1;
                }
                (Some(first), Some(second)) if first == "crate" => {
                    *self
                        .facts
                        .references
                        .entry(crate_target(&second))
                        .or_default() += 1;
                }
                _ => {}
            }
        }
        visit::visit_path(self, path);
    }
}

#[derive(Default)]
struct SelfFields(BTreeSet<String>);

impl<'ast> Visit<'ast> for SelfFields {
    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if let Expr::Path(base) = &*field.base
            && base.path.is_ident("self")
        {
            self.0.insert(match &field.member {
                Member::Named(name) => name.to_string(),
                Member::Unnamed(index) => index.index.to_string(),
            });
        }
        visit::visit_expr_field(self, field);
    }
}

fn production_fields(fields: &Fields) -> impl Iterator<Item = (String, &syn::Field)> {
    fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !test_only(&field.attrs))
        .map(|(index, field)| {
            let name = field
                .ident
                .as_ref()
                .map_or_else(|| index.to_string(), ToString::to_string);
            (name, field)
        })
}

fn is_test(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test")
    })
}

fn owner(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        Type::Reference(reference) => owner(&reference.elem),
        Type::Group(group) => owner(&group.elem),
        Type::Paren(paren) => owner(&paren.elem),
        _ => None,
    }
}

fn use_target(tree: &UseTree) -> Option<String> {
    let UseTree::Path(path) = tree else {
        return None;
    };
    let first = path.ident.to_string();
    if first.starts_with("nocterm_") {
        return Some(first);
    }
    if first != "crate" {
        return None;
    }
    match &*path.tree {
        UseTree::Path(child) => Some(crate_target(&child.ident.to_string())),
        UseTree::Name(child) => Some(crate_target(&child.ident.to_string())),
        UseTree::Rename(child) => Some(crate_target(&child.ident.to_string())),
        _ => None,
    }
}

/// `crate::thread` for a module; items declared or reexported at the crate
/// root (`crate::Installed`) are one `crate::(root)` target.
fn crate_target(name: &str) -> String {
    if name.starts_with(char::is_uppercase) {
        "crate::(root)".into()
    } else {
        format!("crate::{name}")
    }
}

fn lines(span: proc_macro2::Span) -> usize {
    span.end().line.saturating_sub(span.start().line) + 1
}
