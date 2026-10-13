use super::launch::SandboxLauncher;
use super::*;
use nocterm_ai::sandbox::{SandboxLaunchError, SandboxPolicy};
mod native;

#[test]
fn launch_uses_its_fresh_environment_instead_of_probe_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let policy = SandboxPolicy::new(&root, None, &[], &[]);
    let env = BTreeMap::from([("PATH".into(), "/nonexistent".into())]);
    let result = SandboxLauncher::prepare(&policy, &root, Path::new("/bin/true"), &[], &env);
    assert!(matches!(
        result,
        Err(SandboxLaunchError::Missing | SandboxLaunchError::Unsupported)
    ));
}
#[cfg(target_os = "linux")]
#[test]
fn removed_or_unexecutable_bubblewrap_never_falls_back_to_plain_agent() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let bwrap = root.join("bwrap");
    std::fs::write(&bwrap, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&bwrap, std::fs::Permissions::from_mode(0o700)).unwrap();
    let env = BTreeMap::from([("PATH".into(), root.to_string_lossy().into_owned())]);
    assert_eq!(
        availability(&env, Some(&root)),
        Availability::Available(bwrap.clone())
    );
    let policy = SandboxPolicy::new(&root, None, &[], &[]);
    std::fs::set_permissions(&bwrap, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        SandboxLauncher::prepare(&policy, &root, Path::new("/bin/true"), &[], &env),
        Err(SandboxLaunchError::NotExecutable(_))
    ));
    std::fs::remove_file(&bwrap).unwrap();
    assert!(matches!(
        SandboxLauncher::prepare(&policy, &root, Path::new("/bin/true"), &[], &env),
        Err(SandboxLaunchError::Missing)
    ));
}
