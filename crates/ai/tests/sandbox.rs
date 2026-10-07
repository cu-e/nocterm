//! Exercise the filesystem view produced by the real sandbox, not just its argv.
#![cfg(target_os = "linux")]

use std::{fs, io::Write as _, os::unix::fs::symlink, path::Path, process::Command};

use nocterm_ai::sandbox::{Availability, SandboxPolicy, current_availability, file_kind};

fn bubblewrap() -> Option<std::path::PathBuf> {
    let Availability::Available(binary) = current_availability() else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIP actual sandbox: bubblewrap is unavailable on this system"
        );
        return None;
    };
    let probe = Command::new(&binary)
        .args([
            "--ro-bind",
            "/",
            "/",
            "--unshare-pid",
            "--die-with-parent",
            "--new-session",
            "--",
            "/usr/bin/true",
        ])
        .output()
        .expect("start the installed bubblewrap binary");
    if !probe.status.success() {
        let error = String::from_utf8_lossy(&probe.stderr);
        if error.contains("Operation not permitted") || error.contains("Permission denied") {
            let _ = writeln!(
                std::io::stderr(),
                "SKIP actual sandbox: namespace creation is unavailable: {error}"
            );
            return None;
        }
        panic!("bubblewrap probe failed: {error}");
    }
    Some(binary)
}

fn run(binary: &Path, policy: &SandboxPolicy, workdir: &Path, script: &str, paths: &[&Path]) {
    policy.validate_workdir(workdir).unwrap();
    let mut arguments = vec![
        "-eu".into(),
        "-c".into(),
        script.into(),
        "sandbox-test".into(),
    ];
    arguments.extend(paths.iter().map(|path| path.to_str().unwrap().to_owned()));
    let result = Command::new(binary)
        .args(policy.bubblewrap_args(workdir, Path::new("/bin/sh"), &arguments, file_kind))
        .output()
        .expect("run bubblewrap");
    assert!(
        result.status.success(),
        "sandbox failed in {}: status {:?}, stdout {}, stderr {}",
        workdir.display(),
        result.status.code(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
}

fn credentials(home: &Path) {
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::create_dir_all(home.join(".aws")).unwrap();
    fs::write(home.join(".ssh/sentinel"), "ssh-secret").unwrap();
    fs::write(home.join(".aws/sentinel"), "aws-secret").unwrap();
    fs::write(home.join(".netrc"), "netrc-secret").unwrap();
}

#[test]
fn actual_home_and_symlink_workspaces_hide_credentials() {
    let Some(binary) = bubblewrap() else { return };
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    credentials(&home);
    let alias = temporary.path().join("home-alias");
    symlink(&home, &alias).unwrap();
    for (workdir, policy_home) in [(&home, &home), (&alias, &home), (&alias, &alias)] {
        let policy = SandboxPolicy::new(workdir, Some(policy_home), &[], &[]);
        run(
            &binary,
            &policy,
            workdir,
            "test ! -e .ssh/sentinel || exit 41; test ! -e .aws/sentinel || exit 42; test ! -s .netrc || exit 43; printf writable > workspace-marker",
            &[],
        );
        assert_eq!(
            fs::read_to_string(home.join("workspace-marker")).unwrap(),
            "writable"
        );
    }
    assert_eq!(
        fs::read_to_string(home.join(".ssh/sentinel")).unwrap(),
        "ssh-secret"
    );
    assert_eq!(
        fs::read_to_string(home.join(".aws/sentinel")).unwrap(),
        "aws-secret"
    );
    assert_eq!(
        fs::read_to_string(home.join(".netrc")).unwrap(),
        "netrc-secret"
    );
}

#[test]
fn actual_project_workspace_remains_writable() {
    let Some(binary) = bubblewrap() else { return };
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    credentials(&home);
    let project = home.join("project");
    fs::create_dir(&project).unwrap();
    let policy = SandboxPolicy::new(&project, Some(&home), &[], &[]);
    run(
        &binary,
        &policy,
        &project,
        "test ! -e \"$1/.ssh/sentinel\"; test ! -e \"$1/.aws/sentinel\"; printf project-write > result",
        &[&home],
    );
    assert_eq!(
        fs::read_to_string(project.join("result")).unwrap(),
        "project-write"
    );
}

#[test]
fn actual_private_workspace_and_shared_bridge_preserve_their_exceptions() {
    let Some(binary) = bubblewrap() else { return };
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    credentials(&home);
    let private = home.join(".local/state/nocterm");
    let workdir = private.join("agent-workspace");
    let bridge = private.join("bridge");
    fs::create_dir_all(&workdir).unwrap();
    fs::create_dir(&bridge).unwrap();
    fs::create_dir(private.join("vault")).unwrap();
    fs::write(private.join("vault/sentinel"), "vault-secret").unwrap();
    fs::write(bridge.join("marker"), "bridge-readable").unwrap();
    let policy = SandboxPolicy::new(
        &workdir,
        Some(&home),
        std::slice::from_ref(&private),
        std::slice::from_ref(&bridge),
    );
    run(
        &binary,
        &policy,
        &workdir,
        "test ! -e \"$1/vault/sentinel\"; test \"$(cat \"$1/bridge/marker\")\" = bridge-readable; printf private-write > result; if (printf changed > \"$1/bridge/marker\") 2>/dev/null; then exit 9; fi",
        &[&private],
    );
    assert_eq!(
        fs::read_to_string(workdir.join("result")).unwrap(),
        "private-write"
    );
    assert_eq!(
        fs::read_to_string(bridge.join("marker")).unwrap(),
        "bridge-readable"
    );
    assert_eq!(
        fs::read_to_string(private.join("vault/sentinel")).unwrap(),
        "vault-secret"
    );
}

#[test]
fn credential_workspaces_are_rejected_through_direct_and_symlink_paths() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    credentials(&home);
    let secret_project = home.join(".ssh/project");
    fs::create_dir(&secret_project).unwrap();
    let alias = home.join("project-alias");
    symlink(&secret_project, &alias).unwrap();
    let policy = SandboxPolicy::new(&home, Some(&home), &[], &[]);
    for workdir in [
        &home.join(".ssh"),
        &home.join(".aws"),
        &secret_project,
        &alias,
    ] {
        assert!(
            policy.validate_workdir(workdir).is_err(),
            "{} was accepted",
            workdir.display()
        );
    }
    assert!(policy.validate_workdir(&home).is_ok());
}
