//! Cargo dependency edges may activate test-support only through dev dependencies.
use crate::{Metadata, Package};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn check(metadata: &Metadata) -> anyhow::Result<()> {
    let graph = FeatureGraph {
        packages: metadata
            .packages
            .iter()
            .map(|package| (package.name.as_str(), package))
            .collect(),
    };
    for package in &metadata.packages {
        for feature in package.features.keys() {
            if graph.enables_support(&package.name, feature, feature == "test-support") {
                anyhow::bail!(
                    "{} feature {feature} activates dependency or development-only test-support",
                    package.name
                );
            }
        }
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
        {
            let mut enabled = dependency.features.clone();
            if dependency.uses_default_features {
                enabled.push("default".into());
            }
            if enabled
                .iter()
                .any(|feature| graph.enables_support(&dependency.name, feature, false))
            {
                anyhow::bail!(
                    "{} runtime dependency {} activates development-only test-support",
                    package.name,
                    dependency.name
                );
            }
        }
    }
    Ok(())
}

struct FeatureGraph<'a> {
    packages: BTreeMap<&'a str, &'a Package>,
}

impl FeatureGraph<'_> {
    fn enables_support(&self, package: &str, feature: &str, definition: bool) -> bool {
        let mut pending = vec![(package.to_owned(), feature.to_owned(), definition)];
        let mut seen = BTreeSet::new();
        while let Some((owner, feature, definition)) = pending.pop() {
            if feature == "test-support" && !definition {
                return true;
            }
            if !seen.insert((owner.clone(), feature.clone(), definition)) {
                continue;
            }
            let Some(package) = self.packages.get(owner.as_str()) else {
                continue;
            };
            if let Some((binding, child_feature)) = feature.split_once('/') {
                self.forward(
                    package,
                    binding.trim_end_matches('?'),
                    child_feature,
                    &mut pending,
                );
            } else if let Some(binding) = feature.strip_prefix("dep:") {
                self.activate(package, binding, &mut pending);
            } else if let Some(children) = package.features.get(&feature) {
                pending.extend(
                    children
                        .iter()
                        .map(|child| (owner.clone(), child.clone(), false)),
                );
            } else {
                // Cargo also supports implicit optional-dependency features.
                self.activate(package, &feature, &mut pending);
            }
        }
        false
    }

    fn forward(
        &self,
        package: &Package,
        binding: &str,
        feature: &str,
        pending: &mut Vec<(String, String, bool)>,
    ) {
        for dependency in package.dependencies.iter().filter(|dependency| {
            dependency.kind.as_deref() != Some("dev")
                && dependency.rename.as_deref().unwrap_or(&dependency.name) == binding
        }) {
            pending.push((dependency.name.clone(), feature.to_owned(), false));
        }
    }

    fn activate(
        &self,
        package: &Package,
        binding: &str,
        pending: &mut Vec<(String, String, bool)>,
    ) {
        for dependency in package.dependencies.iter().filter(|dependency| {
            dependency.kind.as_deref() != Some("dev")
                && dependency.rename.as_deref().unwrap_or(&dependency.name) == binding
        }) {
            pending.extend(
                dependency
                    .features
                    .iter()
                    .map(|feature| (dependency.name.clone(), feature.clone(), false)),
            );
            if dependency.uses_default_features {
                pending.push((dependency.name.clone(), "default".into(), false));
            }
        }
    }
}

#[cfg(test)]
mod tests;
