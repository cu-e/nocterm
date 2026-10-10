//! Debt entries are exact inventories: growth fails, shrinkage must lower the inventory.
use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Default, Deserialize, Serialize)]
pub(super) struct Counts {
    #[serde(flatten)]
    pub(super) metrics: BTreeMap<String, BTreeMap<String, usize>>,
}

impl Counts {
    pub(super) fn validate(&self) -> anyhow::Result<()> {
        const METRICS: &[&str] = &[
            "struct_fields",
            "pub_fields",
            "impl_spread",
            "globals",
            "global_reads",
            "quit_handlers",
            "string_errors",
            "domain_io",
            "domain_dependencies",
            "complexity_expects",
        ];
        for (metric, entries) in &self.metrics {
            if !METRICS.contains(&metric.as_str()) {
                bail!("unknown baseline metric {metric}");
            }
            for (name, count) in entries {
                if name.trim().is_empty() || *count == 0 {
                    bail!("invalid baseline entry {metric}: {name}: {count}");
                }
            }
        }
        Ok(())
    }
    pub(super) fn set(&mut self, metric: &str, identity: &str, count: usize) {
        self.metrics
            .entry(metric.to_owned())
            .or_default()
            .entry(identity.to_owned())
            .and_modify(|value| *value = (*value).max(count))
            .or_insert(count);
    }
    pub(super) fn bump(&mut self, metric: &str, identity: &str) {
        *self
            .metrics
            .entry(metric.to_owned())
            .or_default()
            .entry(identity.to_owned())
            .or_default() += 1;
    }
    pub(super) fn check(&self, actual: &Self) -> anyhow::Result<()> {
        let mut problems = Vec::new();
        for (metric, entries) in &actual.metrics {
            for (identity, count) in entries {
                let previous = self
                    .metrics
                    .get(metric)
                    .and_then(|entries| entries.get(identity))
                    .copied()
                    .unwrap_or(0);
                if *count > previous {
                    problems.push(format!(
                        "{metric}: {identity}: {previous} -> {count} (growth forbidden)"
                    ));
                } else if *count < previous {
                    problems.push(format!(
                        "{metric}: {identity}: {previous} -> {count}; lower baseline"
                    ));
                }
            }
        }
        for (metric, entries) in &self.metrics {
            for (identity, count) in entries {
                if !actual
                    .metrics
                    .get(metric)
                    .is_some_and(|entries| entries.contains_key(identity))
                {
                    problems.push(format!(
                        "{metric}: {identity}: {count} -> 0; remove baseline entry"
                    ));
                }
            }
        }
        if !problems.is_empty() {
            bail!("architecture debt ratchet:\n{}", problems.join("\n"));
        }
        Ok(())
    }
    pub(super) fn read(path: &Path) -> anyhow::Result<Self> {
        let counts: Self = toml::from_str(&fs::read_to_string(path)?)
            .with_context(|| format!("read {}", path.display()))?;
        counts.validate()?;
        Ok(counts)
    }
    #[cfg(test)]
    pub(super) fn no_growth(&self, previous: &Self) -> anyhow::Result<()> {
        for (metric, entries) in &self.metrics {
            for (name, count) in entries {
                let old = previous
                    .metrics
                    .get(metric)
                    .and_then(|values| values.get(name))
                    .copied()
                    .unwrap_or(0);
                if *count > old {
                    bail!("baseline increase forbidden: {metric}: {name}: {old} -> {count}");
                }
            }
        }
        Ok(())
    }
}
