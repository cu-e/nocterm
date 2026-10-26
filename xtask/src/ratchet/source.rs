//! Loads Cargo's production module trees rather than guessing identity from filenames.
use anyhow::{Context as _, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{Attribute, Item, Meta, Token, UseTree, punctuated::Punctuated};

pub(crate) struct Module {
    pub(crate) name: String,
    pub(crate) package: String,
    pub(crate) layer: String,
    pub(crate) file: PathBuf,
    pub(crate) items: Vec<Item>,
    pub(crate) imports: BTreeMap<String, Vec<String>>,
    pub(crate) io_crates: BTreeMap<String, String>,
    /// Inner attributes of the file (`#![no_std]`); empty for inline modules.
    pub(crate) inner_attributes: Vec<Attribute>,
}

pub(crate) fn load(root: &Path) -> anyhow::Result<Vec<Module>> {
    let metadata = crate::metadata(root)?;
    let mut modules = Vec::new();
    for package in metadata.packages {
        let layer = package
            .metadata
            .pointer("/nocterm/layer")
            .and_then(serde_json::Value::as_str)
            .context("package layer")?;
        for target in &package.targets {
            if !target
                .kind
                .iter()
                .any(|kind| matches!(kind.as_str(), "lib" | "bin" | "proc-macro"))
            {
                continue;
            }
            let name = target.name.replace('-', "_");
            let directory = target.src_path.parent().context("target directory")?;
            let dependencies = package
                .dependencies
                .iter()
                .filter(|dependency| dependency.kind.as_deref() != Some("dev"));
            let external = dependencies
                .clone()
                .map(|dependency| {
                    dependency
                        .rename
                        .as_deref()
                        .unwrap_or(&dependency.name)
                        .replace('-', "_")
                })
                .collect();
            let io_crates = dependencies
                .filter(|dependency| super::IO_DEPENDENCIES.contains(&dependency.name.as_str()))
                .map(|dependency| {
                    (
                        dependency
                            .rename
                            .as_deref()
                            .unwrap_or(&dependency.name)
                            .replace('-', "_"),
                        dependency.name.replace('-', "_"),
                    )
                })
                .collect();
            let mut loader = Loader {
                modules: &mut modules,
                seen: BTreeSet::new(),
                package: &package.name,
                layer,
                io_crates,
                external,
            };
            loader.file(&target.src_path, directory, &name)?;
        }
    }
    modules.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(modules)
}

struct Loader<'a> {
    modules: &'a mut Vec<Module>,
    seen: BTreeSet<(PathBuf, String)>,
    package: &'a str,
    layer: &'a str,
    io_crates: BTreeMap<String, String>,
    external: BTreeSet<String>,
}

impl Loader<'_> {
    fn file(&mut self, file: &Path, directory: &Path, name: &str) -> anyhow::Result<()> {
        let canonical =
            fs::canonicalize(file).with_context(|| format!("module {}", file.display()))?;
        if !self.seen.insert((canonical, name.to_owned())) {
            bail!("recursive module tree at {name}");
        }
        let parsed = syn::parse_file(&fs::read_to_string(file)?)
            .with_context(|| format!("parse {}", file.display()))?;
        if !test_only(&parsed.attrs) {
            super::includes::check(&parsed.items)?;
            super::bindings::check(&parsed.items)?;
            self.items(
                parsed.items,
                parsed.attrs,
                file,
                directory,
                file.parent().context("source directory")?,
                name,
            )?;
        }
        Ok(())
    }

    fn items(
        &mut self,
        items: Vec<Item>,
        inner_attributes: Vec<Attribute>,
        file: &Path,
        directory: &Path,
        attribute_directory: &Path,
        name: &str,
    ) -> anyhow::Result<()> {
        let items: Vec<_> = items
            .into_iter()
            .filter(|item| !test_only(attributes(item)))
            .collect();
        let mut imports = imports(&items);
        for name in &self.external {
            if !imports.contains_key(name) || imports[name] == [name.clone()] {
                imports.insert(name.clone(), vec!["@external".into(), name.clone()]);
            }
        }
        for item in &items {
            let Item::Mod(module) = item else { continue };
            let child = format!("{name}::{}", module.ident);
            if let Some((_, items)) = &module.content {
                let inline_directory = directory.join(module.ident.to_string());
                self.items(
                    items.clone(),
                    Vec::new(),
                    file,
                    &inline_directory,
                    &inline_directory,
                    &child,
                )?;
            } else {
                let path = module_path(module, directory, attribute_directory)?;
                let child_directory = if path.file_name().is_some_and(|name| name == "mod.rs") {
                    path.parent().context("module directory")?.to_path_buf()
                } else {
                    path.with_extension("")
                };
                self.file(&path, &child_directory, &child)?;
            }
        }
        self.modules.push(Module {
            name: name.to_owned(),
            package: self.package.to_owned(),
            layer: self.layer.to_owned(),
            file: file.to_path_buf(),
            items,
            imports,
            io_crates: self.io_crates.clone(),
            inner_attributes,
        });
        Ok(())
    }
}

fn module_path(
    module: &syn::ItemMod,
    directory: &Path,
    attribute_directory: &Path,
) -> anyhow::Result<PathBuf> {
    for attribute in &module.attrs {
        if attribute.path().is_ident("path")
            && let Meta::NameValue(value) = &attribute.meta
            && let syn::Expr::Lit(literal) = &value.value
            && let syn::Lit::Str(path) = &literal.lit
        {
            return Ok(attribute_directory.join(path.value()));
        }
    }
    let leaf = directory.join(format!("{}.rs", module.ident));
    let branch = directory.join(module.ident.to_string()).join("mod.rs");
    match (leaf.is_file(), branch.is_file()) {
        (true, false) => Ok(leaf),
        (false, true) => Ok(branch),
        _ => bail!(
            "module {} must have exactly one source at {}",
            module.ident,
            directory.display()
        ),
    }
}

pub(crate) fn attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::ExternCrate(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        Item::ForeignMod(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Macro(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::TraitAlias(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

pub(crate) fn test_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || attribute
                .path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "test")
            || (attribute.path().is_ident("cfg")
                && match &attribute.meta {
                    Meta::List(list) => syn::parse2::<Meta>(list.tokens.clone())
                        .is_ok_and(|meta| requires_test(&meta)),
                    _ => false,
                })
    })
}

fn requires_test(meta: &Meta) -> bool {
    if meta.path().is_ident("test") {
        return true;
    }
    let Meta::List(list) = meta else { return false };
    let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return false;
    };
    if list.path.is_ident("all") {
        nested.iter().any(requires_test)
    } else if list.path.is_ident("any") {
        !nested.is_empty() && nested.iter().all(requires_test)
    } else {
        false
    }
}

pub(crate) fn imports(items: &[Item]) -> BTreeMap<String, Vec<String>> {
    let mut names = BTreeMap::new();
    for item in items {
        if test_only(attributes(item)) {
            continue;
        }
        match item {
            Item::Use(item) => flatten(&item.tree, &[], &mut names),
            Item::ExternCrate(item) => {
                let alias = item
                    .rename
                    .as_ref()
                    .map(|(_, name)| name)
                    .unwrap_or(&item.ident);
                names.insert(
                    alias.to_string(),
                    vec!["@external".into(), item.ident.to_string()],
                );
            }
            _ => {}
        }
    }
    names
}

pub(crate) fn flatten(
    tree: &UseTree,
    prefix: &[String],
    names: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut path_prefix = prefix.to_vec();
            path_prefix.push(path.ident.to_string());
            flatten(&path.tree, &path_prefix, names);
        }
        UseTree::Name(name) => {
            let mut path = prefix.to_vec();
            if name.ident != "self" {
                path.push(name.ident.to_string());
            }
            if let Some(last) = path.last() {
                names.insert(last.clone(), path);
            }
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.to_vec();
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            if rename.rename != "_" {
                names.insert(rename.rename.to_string(), path);
            }
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                flatten(tree, prefix, names);
            }
        }
        UseTree::Glob(_) => {
            names.insert(format!("*{}", prefix.join("::")), prefix.to_vec());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("nocterm-modules-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn inline_modules_and_path_attributes_keep_rusts_two_relative_directories() {
        let fixture = Fixture::new();
        fixture.write(
            "lib.rs",
            "mod owner; #[cfg(test)] mod missing; mod inline { mod child; }",
        );
        fixture.write(
            "owner.rs",
            "#[path=\"sibling.rs\"] mod renamed; mod nested; pub struct Model;",
        );
        fixture.write("sibling.rs", "pub struct Sibling;");
        fixture.write("owner/nested.rs", "pub struct Nested;");
        fixture.write("inline/child.rs", "pub struct Child;");
        let mut modules = Vec::new();
        let mut loader = Loader {
            modules: &mut modules,
            seen: BTreeSet::new(),
            package: "fixture",
            layer: "feature",
            io_crates: BTreeMap::new(),
            external: BTreeSet::new(),
        };
        loader
            .file(&fixture.0.join("lib.rs"), &fixture.0, "fixture")
            .unwrap();
        let symbols = super::super::symbols::Symbols::new(&modules);
        for name in [
            "fixture::owner::Model",
            "fixture::owner::renamed::Sibling",
            "fixture::owner::nested::Nested",
            "fixture::inline::child::Child",
        ] {
            assert!(symbols.contains(name), "missing {name}");
        }
        assert!(
            !modules
                .iter()
                .any(|module| module.name.ends_with("missing"))
        );
    }

    #[test]
    fn mixed_test_and_production_conditions_remain_in_the_inventory() {
        assert!(test_only(&[syn::parse_quote!(#[cfg(all(test, unix))])]));
        assert!(!test_only(&[
            syn::parse_quote!(#[cfg(any(test, feature="real"))])
        ]));
        assert!(!test_only(&[syn::parse_quote!(#[cfg(not(test))])]));
    }

    #[test]
    fn literal_included_source_is_measured_or_explicitly_rejected() {
        let fixture = Fixture::new();
        let fields = (0..16)
            .map(|index| format!("field{index}:u8"))
            .collect::<Vec<_>>()
            .join(",");
        fixture.write("lib.rs", "include!(\"included.rs\");");
        fixture.write(
            "included.rs",
            &format!("struct State{{{fields}}} impl gpui_kit::Global for State{{}}"),
        );
        let mut modules = Vec::new();
        let mut loader = Loader {
            modules: &mut modules,
            seen: BTreeSet::new(),
            package: "fixture",
            layer: "feature",
            io_crates: BTreeMap::new(),
            external: BTreeSet::new(),
        };
        if let Err(error) = loader.file(&fixture.0.join("lib.rs"), &fixture.0, "fixture") {
            assert!(error.to_string().contains("include"));
            return;
        }
        let symbols = super::super::symbols::Symbols::new(&modules);
        let counts = match super::super::metrics::collect(&modules, &symbols, &BTreeMap::new()) {
            Ok(counts) => counts,
            Err(error) => {
                assert!(error.to_string().contains("include"));
                return;
            }
        };
        assert!(
            counts.metrics.contains_key("struct_fields"),
            "included state escaped inventory"
        );
        assert!(
            counts.metrics.contains_key("globals"),
            "included Global escaped inventory"
        );
    }
    #[test]
    fn conditional_module_paths_cannot_hide_alternate_source_files() {
        for declaration in [
            "#[cfg_attr(feature=\"alternate\",path=\"alternate.rs\")] mod platform;",
            "#[cfg_attr(unix,cfg_attr(feature=\"alternate\",path=\"alternate.rs\"))] mod platform;",
            "#[path=\"alternate\"] mod platform{mod child;}",
        ] {
            let fixture = Fixture::new();
            fixture.write("lib.rs", declaration);
            fixture.write("platform.rs", "struct Small;");
            fixture.write("alternate.rs", "struct Large{a:u8,b:u8,c:u8,d:u8,e:u8,f:u8,g:u8,h:u8,i:u8,j:u8,k:u8,l:u8,m:u8,n:u8,o:u8,p:u8}");
            let mut modules = Vec::new();
            let mut loader = Loader {
                modules: &mut modules,
                seen: BTreeSet::new(),
                package: "fixture",
                layer: "domain",
                io_crates: BTreeMap::new(),
                external: BTreeSet::new(),
            };
            let error = loader
                .file(&fixture.0.join("lib.rs"), &fixture.0, "fixture")
                .unwrap_err();
            assert!(error.to_string().contains("path"), "{declaration}: {error}");
        }
    }
    #[test]
    fn guarded_imports_in_real_module_files_fail_closed() {
        for text in [
            "use std::fs::*; fn read(){let _=read(\"x\");}",
            "use gpui_kit::*; struct Service; impl Global for Service{}",
            "mod exports{pub use std::fs as files;} use exports::*; use files::*; fn read(){let _=read(\"x\");}",
            "#[cfg(unix)] use std::fs::read as action; #[cfg(windows)] use crate::pure::action; fn read(){action(\"x\");}",
        ] {
            let fixture = Fixture::new();
            fixture.write("lib.rs", text);
            let mut modules = Vec::new();
            let mut loader = Loader {
                modules: &mut modules,
                seen: BTreeSet::new(),
                package: "fixture",
                layer: "domain",
                io_crates: BTreeMap::new(),
                external: BTreeSet::new(),
            };
            let result = loader.file(&fixture.0.join("lib.rs"), &fixture.0, "fixture");
            if result.is_err() {
                continue;
            }
            let symbols = super::super::symbols::Symbols::new(&modules);
            assert!(
                super::super::metrics::collect(&modules, &symbols, &BTreeMap::new()).is_err(),
                "{text}"
            );
        }
    }
    #[test]
    fn same_module_platform_files_cannot_overwrite_import_alternatives() {
        let fixture = Fixture::new();
        fixture.write("lib.rs", "#[cfg(unix)] #[path=\"unix.rs\"] mod platform; #[cfg(windows)] #[path=\"windows.rs\"] mod platform;");
        fixture.write(
            "unix.rs",
            "use std::fs::read as action; fn read(){action(\"x\");}",
        );
        fixture.write(
            "windows.rs",
            "use crate::pure::action; fn read(){action(\"x\");}",
        );
        let mut modules = Vec::new();
        let mut loader = Loader {
            modules: &mut modules,
            seen: BTreeSet::new(),
            package: "fixture",
            layer: "domain",
            io_crates: BTreeMap::new(),
            external: BTreeSet::new(),
        };
        loader
            .file(&fixture.0.join("lib.rs"), &fixture.0, "fixture")
            .unwrap();
        let symbols = super::super::symbols::Symbols::new(&modules);
        assert!(super::super::metrics::collect(&modules, &symbols, &BTreeMap::new()).is_err());
    }
}
