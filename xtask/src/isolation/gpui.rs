//! The methods that the gpui receiver types expose (rule I3), read from the vendored source.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::Context as _;
use syn::{
    ImplItem, Type, Visibility,
    visit::{self, Visit},
};

/// Receiver types whose methods hand out host authority.
const RECEIVERS: &[&str] = &["App", "Window", "Context", "AsyncApp", "AsyncWindowContext"];

/// Names of the methods with a `self` receiver that the receiver types expose: the `pub fn`s of
/// their inherent impls, and the methods of every trait defined in the same source that one of
/// them implements. `update_window` and `background_spawn` come from the `AppContext` trait, so
/// the inherent impls alone would leave them outside the list.
pub(super) fn receiver_methods(source: &Path) -> anyhow::Result<BTreeSet<String>> {
    let mut files = Vec::new();
    crate::source_files(source, &mut files)?;
    let mut collector = Collector::default();
    for file in files {
        let parsed = syn::parse_file(&fs::read_to_string(&file)?)
            .with_context(|| format!("parse {}", file.display()))?;
        collector.visit_file(&parsed);
    }
    Ok(collector.finish())
}

#[cfg(test)]
pub(super) fn methods_of(text: &str) -> anyhow::Result<BTreeSet<String>> {
    let mut collector = Collector::default();
    collector.visit_file(&syn::parse_file(text)?);
    Ok(collector.finish())
}

#[derive(Default)]
struct Collector {
    /// Methods of inherent impls on a receiver type.
    names: BTreeSet<String>,
    /// Traits that a receiver type implements.
    implemented: BTreeSet<String>,
    /// Methods with a receiver, by the traits defined in the source.
    defined: BTreeMap<String, BTreeSet<String>>,
}

impl Collector {
    fn finish(mut self) -> BTreeSet<String> {
        for name in &self.implemented {
            if let Some(methods) = self.defined.get(name) {
                self.names.extend(methods.iter().cloned());
            }
        }
        self.names
    }
}

fn is_receiver(ty: &Type) -> bool {
    matches!(ty, Type::Path(ty) if ty
        .path
        .segments
        .last()
        .is_some_and(|segment| RECEIVERS.contains(&segment.ident.to_string().as_str())))
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if is_receiver(&item.self_ty) {
            match &item.trait_ {
                None => {
                    for member in &item.items {
                        if let ImplItem::Fn(function) = member
                            && matches!(function.vis, Visibility::Public(_))
                            && function.sig.receiver().is_some()
                        {
                            self.names.insert(function.sig.ident.to_string());
                        }
                    }
                }
                Some((_, path, _)) => {
                    if let Some(segment) = path.segments.last() {
                        self.implemented.insert(segment.ident.to_string());
                    }
                }
            }
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        let methods = self.defined.entry(item.ident.to_string()).or_default();
        for member in &item.items {
            if let syn::TraitItem::Fn(function) = member
                && function.sig.receiver().is_some()
            {
                methods.insert(function.sig.ident.to_string());
            }
        }
        visit::visit_item_trait(self, item);
    }
}
