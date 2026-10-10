//! Source metrics for production shape, service lookup and domain I/O.
use super::{
    baseline::Counts,
    source::{self, Module},
    symbols::Symbols,
};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Item, Type, TypePath,
    visit::{self, Visit},
};

pub(super) fn collect(
    modules: &[Module],
    symbols: &Symbols,
    dtos: &BTreeMap<String, String>,
) -> anyhow::Result<Counts> {
    super::bindings::check_modules(modules)?;
    symbols.check_globs(modules)?;
    symbols.check_declaration_imports(modules)?;
    let mut counts = Counts::default();
    let mut implementations = BTreeMap::<String, BTreeSet<String>>::new();
    let mut symbols = symbols.clone();
    for module in modules {
        super::includes::check(&module.items)?;
        let mut visitor = Collector {
            module,
            imports: symbols.index_scope(&module.name, &module.items, &module.imports),
            symbols: &mut symbols,
            counts: &mut counts,
            owner: module.name.clone(),
            scope: module.name.clone(),
            blocks: BTreeMap::new(),
            implementations: &mut implementations,
            dtos,
            errors: Vec::new(),
        };
        for item in &module.items {
            visitor.visit_item(item);
        }
        if !visitor.errors.is_empty() {
            anyhow::bail!("{}", visitor.errors.join("\n"));
        }
    }
    for (name, files) in implementations {
        if files.len() > 4 {
            counts.set("impl_spread", &name, files.len());
        }
    }
    Ok(counts)
}

fn record_fields(
    module: &Module,
    scope: &str,
    item: &syn::ItemStruct,
    dtos: &BTreeMap<String, String>,
    counts: &mut Counts,
) {
    let name = format!("{scope}::{}", item.ident);
    let fields: Vec<_> = item
        .fields
        .iter()
        .filter(|field| !source::test_only(&field.attrs))
        .collect();
    if fields.len() > 15 {
        counts.set("struct_fields", &name, fields.len());
    }
    if matches!(module.layer.as_str(), "feature" | "service") && !dtos.contains_key(&name) {
        let public = fields
            .iter()
            .filter(|field| !matches!(field.vis, syn::Visibility::Inherited))
            .count();
        if public > 0 {
            counts.set("pub_fields", &name, public);
        }
    }
}

fn record_impl(
    module: &Module,
    scope: &str,
    imports: &BTreeMap<String, Vec<String>>,
    item: &syn::ItemImpl,
    symbols: &Symbols,
    implementations: &mut BTreeMap<String, BTreeSet<String>>,
    counts: &mut Counts,
) -> anyhow::Result<()> {
    let Type::Path(ty) = ungroup(&item.self_ty) else {
        return Ok(());
    };
    let name = symbols.implementation_type(scope, item, &ty.path, imports);
    let root = module.name.split("::").next().unwrap_or(&module.name);
    let generic_owner = ty.path.get_ident().is_some_and(|ident| {
        item.generics
            .type_params()
            .any(|parameter| parameter.ident == *ident)
    });
    if !generic_owner && name.starts_with(&format!("{root}::")) && !symbols.contains(&name) {
        anyhow::bail!(
            "unresolved local impl owner {name} in {}",
            module.file.display()
        );
    }
    if !generic_owner && symbols.contains(&name) {
        implementations
            .entry(name.clone())
            .or_default()
            .insert(module.file.to_string_lossy().into_owned());
    }
    if let Some((_, path, _)) = &item.trait_
        && symbols
            .resolve_candidates(scope, path, imports)
            .iter()
            .any(|name| name == "gpui_kit::Global" || name == "gpui::Global")
    {
        counts.bump("globals", &name);
    }
    Ok(())
}

struct Collector<'a> {
    module: &'a Module,
    symbols: &'a mut Symbols,
    counts: &'a mut Counts,
    imports: BTreeMap<String, Vec<String>>,
    owner: String,
    scope: String,
    blocks: BTreeMap<String, usize>,
    implementations: &'a mut BTreeMap<String, BTreeSet<String>>,
    dtos: &'a BTreeMap<String, String>,
    errors: Vec<String>,
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if source::test_only(source::attributes(item)) {
            return;
        }
        if matches!(item, Item::Mod(_)) && self.scope == self.module.name {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        record_fields(self.module, &self.scope, item, self.dtos, self.counts);
        visit::visit_item_struct(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let Some((_, items)) = &item.content else {
            self.errors.push(format!(
                "out-of-line local module {} is unsupported in {}",
                item.ident,
                self.module.file.display()
            ));
            return;
        };
        let scope = self.scope.clone();
        let imports = self.imports.clone();
        self.scope = format!("{scope}::{}", item.ident);
        self.symbols.declare_module(&self.scope);
        // Modules have their own lexical imports, rather than a function's locals.
        self.imports = self
            .symbols
            .index_scope(&self.scope, items, &self.module.imports);
        if let Err(error) = self
            .symbols
            .check_scope_globs(self.module, &self.scope, &self.imports)
        {
            self.errors.push(error.to_string());
        }
        for item in items {
            self.visit_item(item);
        }
        self.scope = scope;
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

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let owner = std::mem::replace(
            &mut self.owner,
            format!("{}::{}", self.scope, item.sig.ident),
        );
        visit::visit_item_fn(self, item);
        self.owner = owner;
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let Err(error) = record_impl(
            self.module,
            &self.scope,
            &self.imports,
            item,
            self.symbols,
            self.implementations,
            self.counts,
        ) {
            self.errors.push(error.to_string());
        }
        let name = if let Type::Path(ty) = ungroup(&item.self_ty) {
            self.symbols
                .implementation_type(&self.scope, item, &ty.path, &self.imports)
        } else {
            self.module.name.clone()
        };
        let owner = std::mem::replace(&mut self.owner, name);
        visit::visit_item_impl(self, item);
        self.owner = owner;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if source::test_only(&item.attrs) {
            return;
        }
        let name = format!("{}::{}", self.owner, item.sig.ident);
        let owner = std::mem::replace(&mut self.owner, name);
        visit::visit_impl_item_fn(self, item);
        self.owner = owner;
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let imports = self.imports.clone();
        let scope = self.scope.clone();
        let next = self.blocks.entry(self.owner.clone()).or_default();
        self.scope = format!("{}::block{}", self.owner, *next);
        *next += 1;
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
        let inherited = self.symbols.lexical_imports(&scope, &imports);
        self.imports = self.symbols.index_scope(&self.scope, &items, &inherited);
        if let Err(error) = self
            .symbols
            .check_scope_globs(self.module, &self.scope, &self.imports)
        {
            self.errors.push(error.to_string());
        }
        visit::visit_block(self, block);
        self.imports = imports;
        self.scope = scope;
    }

    fn visit_item_use(&mut self, _item: &'ast syn::ItemUse) {}

    fn visit_type_path(&mut self, ty: &'ast TypePath) {
        if self
            .symbols
            .resolve_candidates(&self.scope, &ty.path, &self.imports)
            .len()
            > 1
        {
            self.errors.push(format!(
                "ambiguous glob type in {}: use an explicit binding",
                self.scope
            ));
        }
        if string_result(
            ty,
            &self.scope,
            self.symbols,
            &self.imports,
            &BTreeMap::new(),
            0,
        ) {
            self.counts.bump("string_errors", &self.module.package);
        }
        visit::visit_type_path(self, ty);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        match call.method.to_string().as_str() {
            "on_app_quit" => self.counts.bump("quit_handlers", &self.owner),
            "global" => self.counts.bump("global_reads", &self.module.package),
            _ => {}
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            for name in self
                .symbols
                .resolve_candidates(&self.scope, &path.path, &self.imports)
            {
                let api_owner = name.rsplit_once("::").map(|(owner, _)| owner);
                if matches!(
                    api_owner,
                    Some(
                        "gpui_kit::App"
                            | "gpui::App"
                            | "gpui_kit::Context"
                            | "gpui::Context"
                            | "gpui_kit::AppContext"
                            | "gpui::AppContext"
                    )
                ) {
                    if name.ends_with("::on_app_quit") {
                        self.counts.bump("quit_handlers", &self.owner);
                    }
                    if name.ends_with("::global") {
                        self.counts.bump("global_reads", &self.module.package);
                    }
                }
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.module.layer == "domain" {
            // A bare identifier matching a dependency can be a local value
            // (e.g. `Image(image)`). Crate roots cannot themselves be values.
            if path.segments.len() == 1
                && self
                    .module
                    .io_crates
                    .contains_key(&path.segments[0].ident.to_string())
            {
                return;
            }
            for mut name in self
                .symbols
                .resolve_candidates(&self.scope, path, &self.imports)
            {
                let first = name.split("::").next().unwrap_or("");
                let dependency = self.module.io_crates.get(first);
                if let Some(canonical) = dependency {
                    name = format!("{canonical}{}", &name[first.len()..]);
                }
                if !pure_network_value(&name)
                    && (dependency.is_some()
                        || core_io(&name)
                        || ["std::fs", "std::net", "std::process"]
                            .iter()
                            .any(|prefix| {
                                name == *prefix || name.starts_with(&format!("{prefix}::"))
                            }))
                {
                    self.counts
                        .bump("domain_io", &format!("{}::{name}", self.module.name));
                }
            }
        }
        visit::visit_path(self, path);
    }
}

fn string_result(
    ty: &TypePath,
    scope: &str,
    symbols: &Symbols,
    imports: &BTreeMap<String, Vec<String>>,
    bindings: &BTreeMap<String, Type>,
    depth: usize,
) -> bool {
    if depth > 24 {
        return false;
    }
    symbols
        .resolve_candidates(scope, &ty.path, imports)
        .iter()
        .any(|name| {
            let arguments = type_arguments(ty);
            if matches!(
                name.as_str(),
                "Result" | "std::result::Result" | "core::result::Result"
            ) {
                return arguments.get(1).is_some_and(|error| {
                    is_string(error, scope, symbols, imports, bindings, depth + 1)
                });
            }
            let Some(aliases) = symbols.aliases.get(name) else {
                return false;
            };
            aliases.iter().any(|alias| {
                let substituted =
                    alias_bindings(alias, &arguments, scope, symbols, imports, bindings);
                if let Type::Path(ty) = ungroup(&alias.ty) {
                    string_result(
                        ty,
                        &alias.scope,
                        symbols,
                        symbols.imports(&alias.scope),
                        &substituted,
                        depth + 1,
                    )
                } else {
                    false
                }
            })
        })
}

fn alias_bindings(
    alias: &super::symbols::Alias,
    arguments: &[&Type],
    scope: &str,
    symbols: &Symbols,
    imports: &BTreeMap<String, Vec<String>>,
    bindings: &BTreeMap<String, Type>,
) -> BTreeMap<String, Type> {
    let mut substituted = BTreeMap::new();
    for (index, (parameter, default)) in alias.parameters.iter().enumerate() {
        let ty = if let Some(argument) = arguments.get(index) {
            qualify(argument, scope, symbols, imports, bindings)
        } else if let Some(default) = default {
            qualify(
                default,
                &alias.scope,
                symbols,
                symbols.imports(&alias.scope),
                &substituted,
            )
        } else {
            continue;
        };
        substituted.insert(parameter.clone(), ty);
    }
    substituted
}

fn ungroup(mut ty: &Type) -> &Type {
    loop {
        match ty {
            Type::Paren(group) => ty = &group.elem,
            Type::Group(group) => ty = &group.elem,
            _ => return ty,
        }
    }
}

fn type_arguments(ty: &TypePath) -> Vec<&Type> {
    let Some(segment) = ty.path.segments.last() else {
        return Vec::new();
    };
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Vec::new();
    };
    arguments
        .args
        .iter()
        .filter_map(|argument| {
            if let syn::GenericArgument::Type(ty) = argument {
                Some(ty)
            } else {
                None
            }
        })
        .collect()
}

fn qualify(
    ty: &Type,
    scope: &str,
    symbols: &Symbols,
    imports: &BTreeMap<String, Vec<String>>,
    bindings: &BTreeMap<String, Type>,
) -> Type {
    let ty = ungroup(ty);
    if let Type::Path(path) = ty
        && let Some(ident) = path.path.get_ident()
        && let Some(bound) = bindings.get(&ident.to_string())
    {
        return bound.clone();
    }
    let mut ty = ty.clone();
    if let Type::Path(path) = &mut ty {
        let name = symbols.resolve(scope, &path.path, imports);
        if symbols.contains(&name)
            || name.starts_with("std::")
            || name.starts_with("core::")
            || name.starts_with("alloc::")
        {
            let arguments = path
                .path
                .segments
                .last()
                .map(|segment| segment.arguments.clone());
            if let Ok(qualified) = syn::parse_str(&name) {
                path.path = qualified;
            }
            if let (Some(segment), Some(arguments)) = (path.path.segments.last_mut(), arguments) {
                segment.arguments = arguments;
            }
        }
        if let Some(syn::PathSegment {
            arguments: syn::PathArguments::AngleBracketed(arguments),
            ..
        }) = path.path.segments.last_mut()
        {
            for argument in &mut arguments.args {
                if let syn::GenericArgument::Type(ty) = argument {
                    *ty = qualify(ty, scope, symbols, imports, bindings);
                }
            }
        }
    }
    ty
}

fn is_string(
    ty: &Type,
    scope: &str,
    symbols: &Symbols,
    imports: &BTreeMap<String, Vec<String>>,
    bindings: &BTreeMap<String, Type>,
    depth: usize,
) -> bool {
    if depth > 24 {
        return false;
    }
    let ty = ungroup(ty);
    let Type::Path(path) = ty else { return false };
    if let Some(ident) = path.path.get_ident()
        && let Some(ty) = bindings.get(&ident.to_string())
    {
        return is_string(ty, scope, symbols, imports, &BTreeMap::new(), depth + 1);
    }
    symbols
        .resolve_candidates(scope, &path.path, imports)
        .iter()
        .any(|name| {
            if matches!(
                name.as_str(),
                "String" | "std::string::String" | "alloc::string::String"
            ) {
                return true;
            }
            symbols.aliases.get(name).is_some_and(|aliases| {
                aliases.iter().any(|alias| {
                    let substituted = alias_bindings(
                        alias,
                        &type_arguments(path),
                        scope,
                        symbols,
                        imports,
                        bindings,
                    );
                    is_string(
                        &alias.ty,
                        &alias.scope,
                        symbols,
                        symbols.imports(&alias.scope),
                        &substituted,
                        depth + 1,
                    )
                })
            })
        })
}

fn core_io(name: &str) -> bool {
    matches!(
        name,
        "nocterm_core::persist::load"
            | "nocterm_core::persist::load_preserving"
            | "nocterm_core::persist::save"
            | "nocterm_core::persist::save_preserving"
            | "nocterm_core::persist::save_text"
            | "nocterm_core::fs::write_private_atomic"
            | "nocterm_core::fs::create_private"
    ) || name
        .strip_prefix("nocterm_core::fs::AtomicPublication::")
        .is_some_and(|method| method == "publish" || method.starts_with("stage"))
}

fn pure_network_value(name: &str) -> bool {
    let Some(path) = name.strip_prefix("std::net::") else {
        return false;
    };
    matches!(
        path.split("::").next(),
        Some(
            "IpAddr"
                | "Ipv4Addr"
                | "Ipv6Addr"
                | "SocketAddr"
                | "SocketAddrV4"
                | "SocketAddrV6"
                | "AddrParseError"
        )
    )
}
