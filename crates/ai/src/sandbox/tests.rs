use super::*;
fn path(original: &str, resolved: &str, kind: PathKind) -> ResolvedPath {
    ResolvedPath {
        original: original.into(),
        resolved: resolved.into(),
        kind,
    }
}
#[test]
fn lexical_and_canonical_credential_descendants_are_refused() {
    let policy = SandboxPolicy::new(
        Path::new("/home/project"),
        Some(Path::new("/alias")),
        &[],
        &[],
    );
    let credential = path("/alias/.ssh", "/real-home/.ssh", PathKind::Directory);
    for workdir in [
        path("/alias/.ssh/work", "/elsewhere", PathKind::Directory),
        path("/safe-alias", "/real-home/.ssh/work", PathKind::Directory),
    ] {
        let facts = ResolvedSandboxPaths {
            workdir,
            credentials: vec![credential.clone()],
            writable: vec![],
            hidden: vec![],
            readable: vec![],
        };
        assert_eq!(policy.validate_workdir(&facts), Err(ProtectedWorkdirError));
        assert!(policy.mount_plan(&facts).is_err());
    }
}
#[test]
fn canonical_policy_mounts_hide_ancestors_then_workspace_then_credentials() {
    let policy = SandboxPolicy::new(Path::new("/home"), Some(Path::new("/home")), &[], &[]);
    let facts = ResolvedSandboxPaths {
        workdir: path("/home", "/home", PathKind::Directory),
        credentials: vec![path("/home/.ssh", "/home/.ssh", PathKind::Directory)],
        writable: vec![
            path("/home/.cache", "/home/.cache", PathKind::Directory),
            path("/state/cache", "/state/cache", PathKind::Directory),
        ],
        hidden: vec![
            path("/state", "/state", PathKind::Directory),
            path("/home/.ssh", "/home/.ssh", PathKind::Directory),
            path("/home/.netrc", "/home/.netrc", PathKind::NonDirectory),
            path("/absent", "/absent", PathKind::Absent),
        ],
        readable: vec![path("/state/socket", "/state/socket", PathKind::Directory)],
    };
    let plan = policy.mount_plan(&facts).unwrap();
    let mounts = plan.mounts();
    assert!(
        !mounts
            .iter()
            .any(|mount| mount.path == Path::new("/state/cache")
                || mount.path == Path::new("/absent"))
    );
    let index = |value: &str| {
        mounts
            .iter()
            .position(|mount| mount.path == Path::new(value))
            .unwrap()
    };
    assert!(index("/home/.cache") < index("/state"));
    assert!(index("/state") < index("/home"));
    assert!(index("/home") < index("/home/.ssh"));
    assert_eq!(mounts[index("/home/.netrc")].access, MountAccess::MaskFile);
    assert_eq!(mounts.last().unwrap().access, MountAccess::ReadOnly);
}
