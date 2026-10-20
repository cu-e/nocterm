use super::*;
use futures::executor::block_on;
use nocterm_ai::{
    sandbox::{PathKind, SandboxLaunchError},
    workspace::IsolationPaths,
};
fn request(path: &std::path::Path, private: bool, isolated: bool) -> WorkdirRequest {
    WorkdirRequest {
        candidate: path.to_owned(),
        create_private: private,
        isolation: isolated.then(|| IsolationPaths {
            private_dirs: vec![],
            shared_dirs: vec![],
            home_state: vec![],
        }),
    }
}
#[test]
fn private_workspace_creation_returns_the_canonical_directory_and_typed_refusals() {
    let temp = tempfile::tempdir().unwrap();
    let resolver = NativeWorkdirResolver::with_home(None);
    let root = temp.path().join("work");
    let prepared = block_on(resolver.prepare(request(&root, true, false))).unwrap();
    assert_eq!(prepared.root, std::fs::canonicalize(&root).unwrap());
    assert!(prepared.sandbox.is_none());
    for path in [
        std::path::Path::new("relative"),
        &temp.path().join("missing"),
    ] {
        assert!(matches!(
            block_on(resolver.prepare(request(path, false, false))),
            Err(WorkdirError::InvalidDirectory)
        ));
    }
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert!(matches!(
        block_on(resolver.prepare(request(&file, false, false))),
        Err(WorkdirError::InvalidDirectory)
    ));
}
#[cfg(unix)]
#[test]
fn direct_and_symlinked_credential_roots_fail_before_preparation_succeeds() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let secret = home.join(".ssh/project");
    std::fs::create_dir_all(&secret).unwrap();
    let alias = temp.path().join("alias");
    symlink(&secret, &alias).unwrap();
    let resolver = NativeWorkdirResolver::with_home(Some(home));
    for root in [&secret, &alias] {
        assert!(matches!(
            block_on(resolver.prepare(request(root, false, true))),
            Err(WorkdirError::Protected(_))
        ));
    }
}
#[cfg(unix)]
#[test]
fn symlinked_home_and_private_roots_share_physical_identity_and_mask_order() {
    use nocterm_ai::sandbox::MountAccess;
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let physical = temp.path().join("real-home");
    let private = physical.join("state");
    let work = private.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir(physical.join(".ssh")).unwrap();
    std::fs::create_dir(physical.join(".cache")).unwrap();
    let alias = temp.path().join("home");
    symlink(&physical, &alias).unwrap();
    let resolver = NativeWorkdirResolver::with_home(Some(alias.clone()));
    let input = WorkdirRequest {
        candidate: alias.join("state/work"),
        create_private: false,
        isolation: Some(IsolationPaths {
            private_dirs: vec![alias.join("state")],
            shared_dirs: vec![],
            home_state: vec![".agent".into()],
        }),
    };
    let prepared = block_on(resolver.prepare(input)).unwrap();
    assert_eq!(prepared.root, work);
    let policy = prepared.sandbox.unwrap();
    assert!(policy.writable.contains(&alias.join(".agent")));
    let facts = crate::sandbox::resolution::PathResolver::resolve(&policy, &work).unwrap();
    assert!(
        facts
            .hidden
            .iter()
            .any(|path| path.resolved == private && path.kind == PathKind::Directory)
    );
    let plan = policy.mount_plan(&facts).unwrap();
    let mounts = plan.mounts();
    let private_index = mounts
        .iter()
        .position(|mount| mount.path == private)
        .unwrap();
    let work_index = mounts.iter().position(|mount| mount.path == work).unwrap();
    assert!(private_index < work_index);
    assert_eq!(mounts[private_index].access, MountAccess::MaskDirectory);
}
#[cfg(unix)]
#[test]
fn changed_workspace_alias_is_refused_before_any_sandbox_program_can_start() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("work");
    let other = temp.path().join("other");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&other).unwrap();
    let resolver = NativeWorkdirResolver::with_home(None);
    let prepared = block_on(resolver.prepare(request(&root, false, true))).unwrap();
    std::fs::remove_dir(&root).unwrap();
    symlink(&other, &root).unwrap();
    let error = crate::sandbox::SandboxLauncher::arguments(
        prepared.sandbox.as_ref().unwrap(),
        &prepared.root,
        std::path::Path::new("/bin/true"),
        &[],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        SandboxLaunchError::Workspace(WorkdirError::ChangedSincePrepared)
    ));
}
#[cfg(unix)]
#[test]
fn unreadable_native_credential_resolution_retains_its_io_cause_and_never_succeeds() {
    use std::{error::Error as _, io::Write as _, os::unix::fs::PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = temp.path().join("work");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o0)).unwrap();
    let permission_probe = std::fs::metadata(home.join(".ssh"));
    let result = block_on(
        NativeWorkdirResolver::with_home(Some(home.clone())).prepare(request(&root, false, true)),
    );
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let denied = match permission_probe {
        Ok(_) => {
            let _ = writeln!(
                std::io::stderr(),
                "SKIP permission refusal: independent OS probe bypassed directory permissions"
            );
            return;
        }
        Err(error) => error,
    };
    assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied);
    let error = result.expect_err("native preparation must reject independently proven denial");
    let source = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
    let clone = error.clone();
    assert!(std::ptr::eq(
        source,
        clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
    ));
}

#[cfg(unix)]
#[test]
fn looping_private_alias_is_a_typed_resolution_failure_instead_of_a_mount_fallback() {
    use std::{error::Error as _, os::unix::fs::symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("work");
    let loop_path = temp.path().join("loop");
    std::fs::create_dir(&root).unwrap();
    symlink("loop", &loop_path).unwrap();
    let input = WorkdirRequest {
        candidate: root,
        create_private: false,
        isolation: Some(IsolationPaths {
            private_dirs: vec![loop_path],
            shared_dirs: Vec::new(),
            home_state: Vec::new(),
        }),
    };
    let error = block_on(NativeWorkdirResolver::with_home(None).prepare(input)).unwrap_err();
    assert!(matches!(error, WorkdirError::Resolve { .. }));
    let source = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert!(source.raw_os_error().is_some());
    let clone = error.clone();
    assert!(std::ptr::eq(
        source,
        clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
    ));
    assert_eq!(error.to_string(), clone.to_string());
}

#[test]
fn private_workspace_creation_refusal_preserves_the_native_cause_and_original_file() {
    use std::error::Error;
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("existing-file");
    std::fs::write(&parent, b"keep").unwrap();
    let expected = std::fs::create_dir_all(&parent).unwrap_err();
    let candidate = parent.join("workspace");
    let error =
        block_on(NativeWorkdirResolver::with_home(None).prepare(request(&candidate, true, false)))
            .unwrap_err();
    assert!(matches!(error, WorkdirError::CreatePrivate { .. }));
    let native = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(native.kind(), expected.kind());
    assert!(native.raw_os_error().is_some());
    let clone = error.clone();
    assert!(std::ptr::eq(
        native,
        clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
    ));
    assert_eq!(std::fs::read(parent).unwrap(), b"keep");
}
