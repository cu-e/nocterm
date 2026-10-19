//! Native sandbox facts and launch projection; policy remains in AI.
#[cfg(test)]
mod launch;
pub(crate) mod resolution;
#[cfg(test)]
pub(crate) use launch::SandboxLauncher;
#[cfg(test)]
mod tests;
use nocterm_ai::sandbox::{Availability, SandboxProbe};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Default)]
pub struct NativeSandboxProbe;
impl SandboxProbe for NativeSandboxProbe {
    fn availability(&self) -> Availability {
        availability(
            &std::env::vars().collect(),
            std::env::current_dir().ok().as_deref(),
        )
    }
}
pub(crate) fn availability(env: &BTreeMap<String, String>, cwd: Option<&Path>) -> Availability {
    if !cfg!(target_os = "linux") {
        return Availability::Unsupported;
    }
    let Some(path) = env.get("PATH") else {
        return Availability::Missing;
    };
    for directory in std::env::split_paths(path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let directory = if directory.is_absolute() {
            directory
        } else {
            cwd.map_or(directory.clone(), |cwd| cwd.join(directory))
        };
        let candidate = directory.join("bwrap");
        if std::fs::metadata(&candidate).is_ok_and(|metadata| metadata.is_file()) {
            return Availability::Available(candidate);
        }
    }
    Availability::Missing
}
