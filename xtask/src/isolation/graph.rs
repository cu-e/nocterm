//! Dependency-graph rules on a full `cargo metadata` resolve: the closure allowlist (I1), the
//! provider dependents (I5) and the compile-out of the user-plugin track.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, bail};
use serde::Deserialize;

use super::policy::{Kind, Policy};

/// Proc-macro crates a row may depend on directly.
const PROC_MACROS: &[&str] = &["serde_derive", "thiserror-impl"];

#[derive(Deserialize)]
struct Full {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    #[serde(default)]
    targets: Vec<Target>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Target {
    kind: Vec<String>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    #[serde(default)]
    deps: Vec<NodeDependency>,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize)]
struct NodeDependency {
    pkg: String,
    dep_kinds: Vec<DependencyKind>,
}

#[derive(Deserialize)]
struct DependencyKind {
    kind: Option<String>,
}

/// A resolved workspace, indexed for closure queries. Dev-dependencies never count.
pub(super) struct Graph {
    full: Full,
    by_id: BTreeMap<String, usize>,
    nodes: BTreeMap<String, usize>,
}

impl Graph {
    pub(super) fn read(root: &Path, arguments: &[&str]) -> anyhow::Result<Self> {
        let output = Command::new("cargo")
            .args(["metadata", "--format-version", "1"])
            .args(arguments)
            .current_dir(root)
            .output()?;
        if !output.status.success() {
            bail!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Self::parse(&output.stdout)
    }

    pub(super) fn parse(bytes: &[u8]) -> anyhow::Result<Self> {
        let full: Full = serde_json::from_slice(bytes).context("cargo metadata")?;
        let by_id = full
            .packages
            .iter()
            .enumerate()
            .map(|(index, package)| (package.id.clone(), index))
            .collect();
        let nodes = full
            .resolve
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect();
        Ok(Self { full, by_id, nodes })
    }

    fn member(&self, name: &str) -> Option<&Package> {
        self.full.packages.iter().find(|package| {
            package.name == name && self.full.workspace_members.contains(&package.id)
        })
    }

    fn named(&self, name: &str) -> Option<&Package> {
        self.member(name).or_else(|| {
            self.full
                .packages
                .iter()
                .find(|package| package.name == name)
        })
    }

    pub(super) fn package_directory(&self, name: &str) -> anyhow::Result<PathBuf> {
        let package = self
            .named(name)
            .with_context(|| format!("package {name} is not in the dependency graph"))?;
        Ok(package
            .manifest_path
            .parent()
            .context("package directory")?
            .to_path_buf())
    }

    /// Names of every package a package reaches through normal and build dependencies.
    fn closure(&self, id: &str) -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(&node) = self.nodes.get(id) else {
                continue;
            };
            for dependency in &self.full.resolve.nodes[node].deps {
                if dependency
                    .dep_kinds
                    .iter()
                    .any(|kind| kind.kind.as_deref() != Some("dev"))
                {
                    pending.push(&dependency.pkg);
                }
            }
        }
        seen.into_iter()
            .filter(|seen| *seen != id)
            .filter_map(|id| self.by_id.get(id))
            .map(|&index| self.full.packages[index].name.as_str())
            .collect()
    }

    fn features(&self, name: &str) -> BTreeSet<&str> {
        self.full
            .packages
            .iter()
            .filter(|package| package.name == name)
            .filter_map(|package| self.nodes.get(&package.id))
            .flat_map(|&node| &self.full.resolve.nodes[node].features)
            .map(String::as_str)
            .collect()
    }

    fn is_proc_macro(&self, name: &str) -> bool {
        self.named(name).is_some_and(|package| {
            package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|kind| kind == "proc-macro"))
        })
    }
}

/// I1 and the build-script and proc-macro bans of I4, for every row present in the workspace.
pub(super) fn closure_problems(policy: &Policy, graph: &Graph) -> BTreeSet<String> {
    let mut problems = BTreeSet::new();
    let gpui = graph
        .named(&policy.gpui)
        .map(|package| graph.closure(&package.id))
        .unwrap_or_default();
    for (name, row) in &policy.crates {
        let Some(package) = graph.member(name) else {
            continue;
        };
        if package
            .targets
            .iter()
            .any(|target| target.kind.iter().any(|kind| kind == "custom-build"))
        {
            problems.insert(format!("I4: {name} has a build script"));
        }
        let external = policy.external_for(row);
        for reached in graph.closure(&package.id) {
            if reached.starts_with("nocterm-") {
                if !row.closure.iter().any(|allowed| allowed == reached) {
                    problems.insert(format!("I1: {name} reaches internal crate {reached}"));
                }
            } else if !external.contains(reached)
                && !(row.kind == Kind::Gpui && (reached == policy.gpui || gpui.contains(reached)))
            {
                problems.insert(format!(
                    "I1: {name} reaches unlisted external crate {reached}"
                ));
            }
        }
        for dependency in package
            .dependencies
            .iter()
            .filter(|d| d.kind.as_deref() != Some("dev"))
        {
            let direct = dependency.name.as_str();
            let allowed = if direct.starts_with("nocterm-") {
                row.closure.iter().any(|allowed| allowed == direct)
            } else {
                row.direct.iter().any(|allowed| allowed == direct)
            };
            if !allowed {
                problems.insert(format!(
                    "I1: {name} depends directly on unlisted crate {direct}"
                ));
            }
            if graph.is_proc_macro(direct) && !PROC_MACROS.contains(&direct) {
                problems.insert(format!("I4: {name} depends on proc-macro crate {direct}"));
            }
        }
    }
    problems
}

/// I5: the direct dependents of the ports crate are a closed set.
pub(super) fn provider_problems(policy: &Policy, graph: &Graph) -> BTreeSet<String> {
    const PORTS: &str = "nocterm-plugin-ports";
    graph
        .full
        .packages
        .iter()
        .filter(|package| graph.full.workspace_members.contains(&package.id))
        .filter(|package| {
            package
                .dependencies
                .iter()
                .any(|d| d.name == PORTS && d.kind.as_deref() != Some("dev"))
        })
        .filter(|package| !policy.providers.allowed.contains(&package.name))
        .map(|package| {
            format!(
                "I5: {} depends on {PORTS} but is not a provider crate",
                package.name
            )
        })
        .collect()
}

/// Compile-out: with default features no user-only crate is reachable from the app.
pub(super) fn compile_out_problems(policy: &Policy, graph: &Graph) -> BTreeSet<String> {
    let mut problems = BTreeSet::new();
    if let Some(app) = graph.member(&policy.app) {
        for reached in graph.closure(&app.id) {
            if policy.compile_out.crates.iter().any(|name| name == reached) {
                problems.insert(format!(
                    "compile-out: {} reaches {reached} without the user-plugins feature",
                    policy.app
                ));
            }
        }
    }
    for (package, feature) in &policy.compile_out.features {
        if graph.features(package).contains(feature.as_str()) {
            problems.insert(format!(
                "compile-out: {package} enables feature {feature} in the default build"
            ));
        }
    }
    problems
}
