//! Resolves Rust paths within the source module tree, keeping same-named types separate.
use super::source::Module;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Item, Path, Type};

#[derive(Clone)]
pub(crate) struct Symbols {
    names: BTreeSet<String>,
    module_scopes: BTreeSet<String>,
    structs: BTreeMap<String, Vec<syn::ItemStruct>>,
    imports: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    pub(super) aliases: BTreeMap<String, Vec<Alias>>,
    pub(super) members: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, PartialEq)]
pub(super) struct Alias {
    pub(super) scope: String,
    pub(super) parameters: Vec<(String, Option<Type>)>,
    pub(super) ty: Type,
}

impl Symbols {
    pub(crate) fn new(modules: &[Module]) -> Self {
        let mut symbols = Self {
            names: BTreeSet::new(),
            structs: BTreeMap::new(),
            module_scopes: modules.iter().map(|module| module.name.clone()).collect(),
            imports: BTreeMap::new(),
            aliases: BTreeMap::new(),
            members: BTreeMap::new(),
        };
        for module in modules {
            symbols.names.insert(module.name.clone());
            symbols
                .imports
                .entry(module.name.clone())
                .or_default()
                .extend(module.imports.clone());
            for item in &module.items {
                symbols.index_item(&module.name, item);
            }
        }
        for module in modules {
            for item in &module.items {
                if let Item::Impl(item) = item {
                    let Type::Path(ty) = item.self_ty.as_ref() else {
                        continue;
                    };
                    let name = symbols.canonical_type(&module.name, &ty.path, &module.imports);
                    for item in &item.items {
                        let member = match item {
                            syn::ImplItem::Fn(i) => i.sig.ident.to_string(),
                            syn::ImplItem::Const(i) => i.ident.to_string(),
                            syn::ImplItem::Type(i) => i.ident.to_string(),
                            _ => continue,
                        };
                        symbols
                            .members
                            .entry(name.clone())
                            .or_default()
                            .insert(member);
                    }
                }
            }
        }
        symbols
    }

    pub(super) fn check_declaration_imports(&self, modules: &[Module]) -> anyhow::Result<()> {
        for module in modules {
            for item in &module.items {
                let Item::Struct(item) = item else { continue };
                let name = item.ident.to_string();
                let Some(import) = module.imports.get(&name) else {
                    continue;
                };
                let path: Path = syn::parse_str(&import.join("::"))?;
                let target = self.resolve(&module.name, &path, &module.imports);
                if !self.structs.contains_key(&target)
                    || target == format!("{}::{name}", module.name)
                {
                    anyhow::bail!(
                        "ambiguous imported and declared type {name}: only verified local concrete struct alternatives are supported"
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn implementation_type(
        &self,
        scope: &str,
        item: &syn::ItemImpl,
        path: &Path,
        imports: &BTreeMap<String, Vec<String>>,
    ) -> String {
        let local = self.resolve(scope, path, &BTreeMap::new());
        if let Some(declarations) = self.structs.get(&local) {
            if declarations
                .iter()
                .any(|declaration| conditions(&declaration.attrs) == conditions(&item.attrs))
            {
                return local;
            }
            if path.get_ident().is_some_and(|name| {
                imports
                    .get(&name.to_string())
                    .is_some_and(|binding| binding != &["@external".to_owned(), local.clone()])
            }) {
                return format!(
                    "{}::ambiguous_impl_owner::{local}",
                    scope.split("::").next().unwrap_or(scope)
                );
            }
        }
        self.canonical_type(scope, path, imports)
    }

    pub(super) fn check_globs(&self, modules: &[Module]) -> anyhow::Result<()> {
        for module in modules {
            self.check_scope_globs(module, &module.name, &module.imports)?;
        }
        Ok(())
    }

    pub(super) fn check_scope_globs(
        &self,
        module: &Module,
        scope: &str,
        imports: &BTreeMap<String, Vec<String>>,
    ) -> anyhow::Result<()> {
        for (key, glob) in imports {
            if !key.starts_with('*') {
                continue;
            }
            let path: Path = syn::parse_str(
                &glob
                    .iter()
                    .filter(|part| part.as_str() != "@external")
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("::"),
            )?;
            let target = self.resolve(scope, &path, imports);
            // GPUI's verified presentation prelude does not export Global or I/O.
            if matches!(target.as_str(), "gpui::prelude" | "gpui_kit::prelude") {
                continue;
            }
            let root = target.split("::").next().unwrap_or("");
            let dependency = module
                .imports
                .get(root)
                .is_some_and(|path| path.first().is_some_and(|part| part == "@external"));
            if !self.contains(&target)
                && (is_external(root) || dependency || module.io_crates.contains_key(root))
            {
                anyhow::bail!(
                    "unsupported external glob {target} in {scope}: import explicit names so architecture ownership is unambiguous"
                );
            }
        }
        Ok(())
    }

    pub(super) fn lexical_imports(
        &self,
        scope: &str,
        imports: &BTreeMap<String, Vec<String>>,
    ) -> BTreeMap<String, Vec<String>> {
        imports
            .iter()
            .map(|(name, path)| {
                let canonical = syn::parse_str::<Path>(&path.join("::"))
                    .map(|path| self.resolve(scope, &path, imports));
                let value = match canonical {
                    Ok(value)
                        if self.contains(&value)
                            || guarded_external_target(&value)
                            || value.starts_with("nocterm_") =>
                    {
                        vec!["@external".into(), value]
                    }
                    _ => path.clone(),
                };
                (name.clone(), value)
            })
            .collect()
    }

    pub(super) fn declare_module(&mut self, scope: &str) {
        self.module_scopes.insert(scope.into());
    }

    pub(super) fn index_scope(
        &mut self,
        scope: &str,
        items: &[Item],
        inherited: &BTreeMap<String, Vec<String>>,
    ) -> BTreeMap<String, Vec<String>> {
        let mut imports = inherited.clone();
        imports.extend(super::source::imports(items));
        self.names.insert(scope.into());
        for item in items {
            if super::source::test_only(super::source::attributes(item)) {
                continue;
            }
            self.index_item(scope, item);
            let ident = match item {
                Item::Struct(i) => Some(&i.ident),
                Item::Enum(i) => Some(&i.ident),
                Item::Type(i) => Some(&i.ident),
                Item::Trait(i) => Some(&i.ident),
                Item::Mod(i) => {
                    self.names.insert(format!("{scope}::{}", i.ident));
                    Some(&i.ident)
                }
                _ => None,
            };
            if let Some(ident) = ident {
                if let Item::Struct(item) = item
                    && !conditions(&item.attrs).is_empty()
                    && inherited.contains_key(&ident.to_string())
                {
                    continue;
                }
                imports.insert(
                    ident.to_string(),
                    vec!["@external".into(), format!("{scope}::{ident}")],
                );
            }
        }
        self.imports.insert(scope.into(), imports.clone());
        imports
    }

    fn index_item(&mut self, module: &str, item: &Item) {
        if let Item::Struct(item) = item {
            self.structs
                .entry(format!("{module}::{}", item.ident))
                .or_default()
                .push(item.clone());
        }
        let (ident, members) = match item {
            Item::Struct(i) => (
                i.ident.to_string(),
                i.fields
                    .iter()
                    .filter_map(|f| f.ident.as_ref().map(ToString::to_string))
                    .collect(),
            ),
            Item::Enum(i) => (
                i.ident.to_string(),
                i.variants.iter().map(|v| v.ident.to_string()).collect(),
            ),
            Item::Trait(i) => (
                i.ident.to_string(),
                i.items
                    .iter()
                    .filter_map(|i| match i {
                        syn::TraitItem::Fn(i) => Some(i.sig.ident.to_string()),
                        syn::TraitItem::Type(i) => Some(i.ident.to_string()),
                        syn::TraitItem::Const(i) => Some(i.ident.to_string()),
                        _ => None,
                    })
                    .collect(),
            ),
            Item::Fn(i) => (i.sig.ident.to_string(), BTreeSet::new()),
            Item::Const(i) => (i.ident.to_string(), BTreeSet::new()),
            Item::Static(i) => (i.ident.to_string(), BTreeSet::new()),
            Item::Type(i) => {
                let parameters = i
                    .generics
                    .type_params()
                    .map(|parameter| (parameter.ident.to_string(), parameter.default.clone()))
                    .collect();
                let alias = Alias {
                    scope: module.into(),
                    parameters,
                    ty: *i.ty.clone(),
                };
                let alternatives = self
                    .aliases
                    .entry(format!("{module}::{}", i.ident))
                    .or_default();
                if !alternatives.contains(&alias) {
                    alternatives.push(alias);
                }
                (i.ident.to_string(), BTreeSet::new())
            }
            _ => return,
        };
        let name = format!("{module}::{ident}");
        self.names.insert(name.clone());
        self.members.insert(name, members);
    }

    pub(crate) fn resolve_candidates(
        &self,
        module: &str,
        path: &Path,
        imports: &BTreeMap<String, Vec<String>>,
    ) -> BTreeSet<String> {
        let mut names = BTreeSet::from([self.resolve(module, path, imports)]);
        if let Some(first) = path.segments.first()
            && !imports.contains_key(&first.ident.to_string())
            && !self.names.contains(&format!("{module}::{}", first.ident))
            && !matches!(first.ident.to_string().as_str(), "self" | "super" | "crate")
            && !is_external(&first.ident.to_string())
        {
            let segments: Vec<_> = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            for (key, glob) in imports.iter().filter(|(key, _)| key.starts_with('*')) {
                let expanded: Vec<_> = glob.iter().chain(&segments).cloned().collect();
                let mut isolated = imports.clone();
                isolated.retain(|name, _| !name.starts_with('*') || name == key);
                let candidate =
                    self.resolve_segments(module, &expanded, &isolated, 0, &mut BTreeSet::new());
                if self.contains(&candidate) || guarded_external_target(&candidate) {
                    names.insert(normalize_prelude(candidate));
                }
            }
        }
        let mut result = BTreeSet::new();
        for name in names {
            self.expand_associated(&name, &mut BTreeSet::new(), &mut result);
        }
        result
    }

    fn expand_associated(
        &self,
        name: &str,
        seen: &mut BTreeSet<String>,
        result: &mut BTreeSet<String>,
    ) {
        if !seen.insert(name.into()) {
            return;
        }
        if let Some((owner, member)) = name.rsplit_once("::")
            && let Some(aliases) = self.aliases.get(owner)
        {
            for alias in aliases {
                let Type::Path(ty) = ungroup(&alias.ty) else {
                    continue;
                };
                let mut path = ty.path.clone();
                if let Ok(segment) = syn::parse_str(member) {
                    path.segments.push(segment);
                }
                let candidate = self.resolve(&alias.scope, &path, self.imports(&alias.scope));
                self.expand_associated(&candidate, seen, result);
            }
        } else {
            result.insert(name.into());
        }
    }

    pub(super) fn resolve(
        &self,
        module: &str,
        path: &Path,
        imports: &BTreeMap<String, Vec<String>>,
    ) -> String {
        let segments: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        self.resolve_segments(module, &segments, imports, 0, &mut BTreeSet::new())
    }

    fn resolve_segments(
        &self,
        module: &str,
        path: &[String],
        imports: &BTreeMap<String, Vec<String>>,
        depth: usize,
        visited: &mut BTreeSet<(String, String)>,
    ) -> String {
        if path.is_empty() || depth > 32 || !visited.insert((module.into(), path.join("::"))) {
            return path.join("::");
        }
        let first = &path[0];
        if first == "@external" {
            return self.follow_reexport(&path[1..].join("::"), depth, visited);
        }
        if let Some(alias) = imports.get(first)
            && alias != &path[..1]
        {
            let expanded: Vec<_> = alias.iter().chain(&path[1..]).cloned().collect();
            return self.resolve_segments(module, &expanded, imports, depth + 1, visited);
        }
        let rust_module = self
            .module_scopes
            .iter()
            .filter(|scope| module == scope.as_str() || module.starts_with(&format!("{scope}::")))
            .max_by_key(|scope| scope.len())
            .map(String::as_str)
            .unwrap_or(module);
        let context = if matches!(first.as_str(), "self" | "super" | "crate") {
            rust_module
        } else {
            module
        };
        let mut base: Vec<String> = context.split("::").map(str::to_owned).collect();
        let mut rest = path;
        if first == "crate" {
            base.truncate(1);
            rest = &path[1..];
        } else if first == "self" {
            rest = &path[1..];
        } else if first == "super" {
            while rest.first().is_some_and(|s| s == "super") {
                base.pop();
                rest = &rest[1..];
            }
        } else if self.names.contains(&format!("{module}::{first}")) {
            // Local declaration or module.
        } else if first == base.first().unwrap_or(first)
            || is_external(first)
            || first.starts_with("nocterm_")
        {
            base.clear();
        } else {
            for (key, glob) in imports {
                if !key.starts_with('*') {
                    continue;
                }
                let expanded: Vec<_> = glob.iter().chain(path).cloned().collect();
                let candidate =
                    self.resolve_segments(module, &expanded, imports, depth + 1, visited);
                if self.names.contains(&candidate) || guarded_external_target(&candidate) {
                    return normalize_prelude(candidate);
                }
            }
            // The implicit prelude loses to declarations and explicit/glob
            // imports. Qualified local paths never reach this fallback.
            if let Some(owner) = prelude_type(first) {
                return std::iter::once(owner.to_owned())
                    .chain(path[1..].iter().cloned())
                    .collect::<Vec<_>>()
                    .join("::");
            }
            // Unknown paths remain module-qualified: they never collide across modules.
        }
        base.extend_from_slice(rest);
        let candidate = base.join("::");
        self.follow_reexport(&candidate, depth, visited)
    }

    fn follow_reexport(
        &self,
        candidate: &str,
        depth: usize,
        visited: &mut BTreeSet<(String, String)>,
    ) -> String {
        if depth > 32 || self.names.contains(candidate) {
            return candidate.to_owned();
        }
        let Some((scope, name)) = candidate.rsplit_once("::") else {
            return candidate.to_owned();
        };
        if let Some(aliases) = self.aliases.get(scope) {
            if aliases.len() > 1 {
                return candidate.to_owned();
            }
            let alias = &aliases[0];
            let Type::Path(ty) = ungroup(&alias.ty) else {
                return candidate.to_owned();
            };
            let path: Vec<_> = ty
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .chain([name.to_owned()])
                .collect();
            return self.resolve_segments(
                &alias.scope,
                &path,
                self.imports(&alias.scope),
                depth + 1,
                visited,
            );
        }
        let expanded_scope = self.follow_reexport(scope, depth + 1, visited);
        if expanded_scope != scope {
            return self.follow_reexport(&format!("{expanded_scope}::{name}"), depth + 1, visited);
        }
        if let Some(imports) = self.imports.get(scope) {
            if let Some(path) = imports.get(name) {
                return self.resolve_segments(scope, path, imports, depth + 1, visited);
            }
            for (key, glob) in imports {
                if key.starts_with('*') && depth < 32 {
                    let expanded: Vec<_> = glob.iter().cloned().chain([name.to_owned()]).collect();
                    let value =
                        self.resolve_segments(scope, &expanded, imports, depth + 1, visited);
                    if self.names.contains(&value) || guarded_external_target(&value) {
                        return normalize_prelude(value);
                    }
                }
            }
        }
        candidate.to_owned()
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    pub(super) fn imports(&self, module: &str) -> &BTreeMap<String, Vec<String>> {
        &self.imports[module]
    }

    pub(super) fn canonical_type(
        &self,
        module: &str,
        path: &Path,
        imports: &BTreeMap<String, Vec<String>>,
    ) -> String {
        let candidates = self.resolve_candidates(module, path, imports);
        if candidates.len() > 1 {
            return format!(
                "{}::ambiguous_impl_owner::{}",
                module.split("::").next().unwrap_or(module),
                path.segments.last().unwrap().ident
            );
        }
        let mut name = candidates
            .into_iter()
            .next()
            .unwrap_or_else(|| self.resolve(module, path, imports));
        let mut visited = BTreeSet::new();
        while visited.insert(name.clone()) {
            let Some(aliases) = self.aliases.get(&name) else {
                break;
            };
            let candidates: BTreeSet<_> = aliases
                .iter()
                .filter_map(|alias| {
                    let Type::Path(ty) = ungroup(&alias.ty) else {
                        return None;
                    };
                    Some(self.resolve(&alias.scope, &ty.path, self.imports(&alias.scope)))
                })
                .collect();
            if candidates.len() > 1 {
                let root = module.split("::").next().unwrap_or(module);
                return format!("{root}::ambiguous_impl_owner::{name}");
            }
            let Some(candidate) = candidates.into_iter().next() else {
                break;
            };
            name = candidate;
        }
        name
    }
}

fn is_external(name: &str) -> bool {
    matches!(
        name,
        "std" | "core" | "alloc" | "gpui" | "gpui_kit" | "gpui_component"
    )
}

fn prelude_type(name: &str) -> Option<&'static str> {
    match name {
        "Box" => Some("std::boxed::Box"),
        "Vec" => Some("std::vec::Vec"),
        "String" => Some("std::string::String"),
        "Option" => Some("std::option::Option"),
        "Result" => Some("std::result::Result"),
        "bool" => Some("core::primitive::bool"),
        "char" => Some("core::primitive::char"),
        "str" => Some("core::primitive::str"),
        "u8" => Some("core::primitive::u8"),
        "u16" => Some("core::primitive::u16"),
        "u32" => Some("core::primitive::u32"),
        "u64" => Some("core::primitive::u64"),
        "u128" => Some("core::primitive::u128"),
        "usize" => Some("core::primitive::usize"),
        "i8" => Some("core::primitive::i8"),
        "i16" => Some("core::primitive::i16"),
        "i32" => Some("core::primitive::i32"),
        "i64" => Some("core::primitive::i64"),
        "i128" => Some("core::primitive::i128"),
        "isize" => Some("core::primitive::isize"),
        "f32" => Some("core::primitive::f32"),
        "f64" => Some("core::primitive::f64"),
        _ => None,
    }
}

fn guarded_external_target(name: &str) -> bool {
    let root = name.split("::").next().unwrap_or("");
    matches!(root, "std" | "core" | "alloc" | "gpui" | "gpui_kit")
        || super::IO_DEPENDENCIES
            .iter()
            .any(|dependency| dependency.replace('-', "_") == root)
}

fn normalize_prelude(name: String) -> String {
    for crate_name in ["gpui", "gpui_kit"] {
        if let Some(member) = name.strip_prefix(&format!("{crate_name}::prelude::"))
            && matches!(member.split("::").next(), Some("Context" | "AppContext"))
        {
            return format!("{crate_name}::{member}");
        }
    }
    name
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

fn conditions(attributes: &[syn::Attribute]) -> Vec<&syn::Meta> {
    attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
        })
        .map(|attribute| &attribute.meta)
        .collect()
}
