//! Native sandbox facts and launch projection; policy remains in AI.
mod launch;
pub(crate) mod resolution;
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
    lookup(env, cwd, false).unwrap_or(Availability::Missing)
}
pub(crate) fn strict_availability(
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<Availability, nocterm_ai::workspace::WorkdirError> {
    lookup(env, Some(cwd), true)
}
fn lookup(
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
    strict: bool,
) -> Result<Availability, nocterm_ai::workspace::WorkdirError> {
    if !cfg!(target_os = "linux") {
        return Ok(Availability::Unsupported);
    }
    let Some(path) = env.get("PATH") else {
        return Ok(Availability::Missing);
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
        match std::fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => return Ok(Availability::Available(candidate)),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(source) if strict => {
                return Err(nocterm_ai::workspace::WorkdirError::resolve(
                    candidate, source,
                ));
            }
            Err(_) => {}
        }
    }
    Ok(Availability::Missing)
}
