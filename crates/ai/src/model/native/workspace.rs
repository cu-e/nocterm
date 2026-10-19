//! Pure workspace and sandbox DTO construction keeps every path receipt.
use super::{NativeList, NativeOwned, path::NativePathError};
use crate::{
    model::json::deferred::{DecodeLedger, JsonDecodeError},
    sandbox::{Mount, MountAccess, PathKind, ResolvedPath, ResolvedSandboxPaths, SandboxPolicy},
    workspace::PreparedWorkspace,
};
use std::path::{Path, PathBuf};
pub fn prepared_workspace(
    root: NativeOwned<PathBuf>,
    sandbox: NativeOwned<Option<SandboxPolicy>>,
) -> Result<NativeOwned<PreparedWorkspace>, JsonDecodeError> {
    let ((root, sandbox), allocation) = root.join(sandbox)?.into_parts();
    Ok(NativeOwned::from_parts(
        PreparedWorkspace { root, sandbox },
        allocation,
    ))
}
pub fn sandbox_policy(
    ledger: &DecodeLedger,
    workdir: &Path,
    home: Option<&Path>,
    private: &[PathBuf],
    shared: &[PathBuf],
) -> Result<NativeOwned<SandboxPolicy>, NativePathError> {
    let hidden_count = private
        .len()
        .checked_add(home.map_or(0, |_| crate::sandbox::credential_home_entries().len()))
        .ok_or(JsonDecodeError::Arithmetic)?;
    let mut writable = NativeList::new(
        ledger,
        1 + home.map_or(0, |_| crate::sandbox::agent_state_home_entries().len()),
    )?;
    let mut hidden = NativeList::new(ledger, hidden_count)?;
    let mut readable = NativeList::new(ledger, shared.len())?;
    let mut credentials = NativeList::new(
        ledger,
        home.map_or(0, |_| crate::sandbox::credential_home_entries().len()),
    )?;
    writable.push(NativeOwned::<PathBuf>::copy_path(ledger, workdir)?)?;
    for path in private {
        hidden.push(NativeOwned::<PathBuf>::copy_path(ledger, path)?)?;
    }
    for path in shared {
        readable.push(NativeOwned::<PathBuf>::copy_path(ledger, path)?)?;
    }
    if let Some(home) = home {
        for relative in crate::sandbox::credential_home_entries() {
            let path = NativeOwned::<PathBuf>::join_path(ledger, home, relative)?;
            hidden.push(NativeOwned::<PathBuf>::copy_path(ledger, path.as_path())?)?;
            credentials.push(path)?;
        }
        for relative in crate::sandbox::agent_state_home_entries() {
            writable.push(NativeOwned::<PathBuf>::join_path(ledger, home, relative)?)?;
        }
    }
    Ok(from_paths(
        writable.finish(),
        hidden.finish(),
        readable.finish(),
        credentials.finish(),
    )?)
}
pub fn from_paths(
    writable: NativeOwned<Vec<PathBuf>>,
    hidden: NativeOwned<Vec<PathBuf>>,
    readable: NativeOwned<Vec<PathBuf>>,
    credentials: NativeOwned<Vec<PathBuf>>,
) -> Result<NativeOwned<SandboxPolicy>, JsonDecodeError> {
    let ((((writable, hidden), readable), credentials), allocation) = writable
        .join(hidden)?
        .join(readable)?
        .join(credentials)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        SandboxPolicy {
            writable,
            hidden,
            readable,
            credentials,
        },
        allocation,
    ))
}
pub fn resolved_path(
    original: NativeOwned<PathBuf>,
    resolved: NativeOwned<PathBuf>,
    kind: PathKind,
) -> Result<NativeOwned<ResolvedPath>, JsonDecodeError> {
    let ((original, resolved), allocation) = original.join(resolved)?.into_parts();
    Ok(NativeOwned::from_parts(
        ResolvedPath {
            original,
            resolved,
            kind,
        },
        allocation,
    ))
}
/// Retire the original spelling physically before returning its exact credit.
pub fn into_resolved_path(
    mut owned: NativeOwned<ResolvedPath>,
) -> Result<NativeOwned<PathBuf>, JsonDecodeError> {
    let retired = owned.allocation.split(owned.value.original.capacity())?;
    let (
        ResolvedPath {
            original, resolved, ..
        },
        allocation,
    ) = owned.into_parts();
    drop(original);
    drop(retired);
    Ok(NativeOwned::from_parts(resolved, allocation))
}
pub fn resolved_sandbox(
    workdir: NativeOwned<ResolvedPath>,
    credentials: NativeOwned<Vec<ResolvedPath>>,
    writable: NativeOwned<Vec<ResolvedPath>>,
    hidden: NativeOwned<Vec<ResolvedPath>>,
    readable: NativeOwned<Vec<ResolvedPath>>,
) -> Result<NativeOwned<ResolvedSandboxPaths>, JsonDecodeError> {
    let (((((workdir, credentials), writable), hidden), readable), allocation) = workdir
        .join(credentials)?
        .join(writable)?
        .join(hidden)?
        .join(readable)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ResolvedSandboxPaths {
            workdir,
            credentials,
            writable,
            hidden,
            readable,
        },
        allocation,
    ))
}
pub fn sandbox_mount(path: NativeOwned<PathBuf>, access: MountAccess) -> NativeOwned<Mount> {
    let (path, allocation) = path.into_parts();
    NativeOwned::from_parts(Mount { path, access }, allocation)
}

#[cfg(test)]
mod tests;
