use super::{resolution, strict_availability};
use nocterm_ai::{
    sandbox::{Availability, MountAccess, SandboxLaunchError, SandboxPolicy},
    workspace::WorkdirError,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) struct SandboxCommand {
    executable: PathBuf,
    arguments: Vec<String>,
}
impl SandboxCommand {
    pub(crate) fn into_parts(self) -> (PathBuf, Vec<String>) {
        (self.executable, self.arguments)
    }
}
pub(crate) struct SandboxLauncher;
impl SandboxLauncher {
    pub(crate) fn prepare(
        policy: &SandboxPolicy,
        workdir: &Path,
        executable: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<SandboxCommand, SandboxLaunchError> {
        let arguments = Self::arguments(policy, workdir, executable, args)?;
        let bwrap = match strict_availability(env, workdir)? {
            Availability::Available(path) => path,
            Availability::Missing => return Err(SandboxLaunchError::Missing),
            Availability::Unsupported => return Err(SandboxLaunchError::Unsupported),
        };
        if !crate::process::executable(&bwrap) {
            return Err(SandboxLaunchError::NotExecutable(bwrap));
        }
        Ok(SandboxCommand {
            executable: bwrap,
            arguments,
        })
    }
    pub(crate) fn arguments(
        policy: &SandboxPolicy,
        workdir: &Path,
        executable: &Path,
        args: &[String],
    ) -> Result<Vec<String>, SandboxLaunchError> {
        let paths = resolution::PathResolver::resolve(policy, workdir)?;
        policy.validate_workdir(&paths)?;
        if paths.workdir.resolved.as_path() != workdir {
            return Err(WorkdirError::ChangedSincePrepared.into());
        }
        let plan = policy.mount_plan(&paths)?;
        let mut out = [
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--unshare-pid",
            "--die-with-parent",
            "--new-session",
            "--share-net",
        ]
        .map(str::to_owned)
        .to_vec();
        for mount in plan.mounts() {
            let path = mount.path.to_string_lossy().into_owned();
            match mount.access {
                MountAccess::Writable => out.extend(["--bind".into(), path.clone(), path]),
                MountAccess::ReadOnly => out.extend(["--ro-bind".into(), path.clone(), path]),
                MountAccess::MaskDirectory => out.extend(["--tmpfs".into(), path]),
                MountAccess::MaskFile => out.extend(["--ro-bind".into(), "/dev/null".into(), path]),
            }
        }
        out.extend([
            "--chdir".into(),
            workdir.to_string_lossy().into_owned(),
            "--".into(),
            executable.to_string_lossy().into_owned(),
        ]);
        out.extend(args.iter().cloned());
        Ok(out)
    }
}
