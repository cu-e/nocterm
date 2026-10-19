//! The original bubblewrap argv, kept to exercise the real sandbox filesystem view.
use super::resolution;
use nocterm_ai::{
    sandbox::{MountAccess, SandboxLaunchError, SandboxPolicy},
    workspace::WorkdirError,
};
use std::path::Path;

pub(crate) struct SandboxLauncher;
impl SandboxLauncher {
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
