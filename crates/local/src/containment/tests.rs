//! Actual namespace, host-exec and PTY behavior, without a synthetic launcher.
use super::*;
use futures::executor::block_on;
use nocterm_session::{Event, testing::ScriptedConnectionSource};
use std::{fs, os::unix::fs::PermissionsExt as _, time::Duration};

fn limits() -> ContainmentLimits {
    ContainmentLimits::new(256 * 1024 * 1024, 64, 64, 10, 8 * 1024 * 1024).unwrap()
}
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let state = root.path().join("private-state");
    for path in [&workspace, &state] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(state.join("secret"), b"supervisor-only").unwrap();
    (root, workspace, state)
}
#[test]
fn mounting_supervisor_state_or_its_parent_is_refused_before_launch() {
    let (_root, workspace, state) = setup();
    assert!(matches!(
        block_on(ContainedLocal::prepare(
            state.clone(),
            state.clone(),
            limits()
        )),
        Err(ContainmentError::Workspace)
    ));
    let alias = workspace.join("state-alias");
    std::os::unix::fs::symlink(&state, &alias).unwrap();
    assert!(matches!(
        block_on(ContainedLocal::prepare(alias, state.clone(), limits())),
        Err(ContainmentError::Workspace)
    ));
    fs::set_permissions(&workspace, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        block_on(ContainedLocal::prepare(workspace, state, limits())),
        Err(ContainmentError::Workspace)
    ));
}

#[test]
#[ignore = "requires bubblewrap, prlimit and enabled Linux user namespaces"]
fn contained_execution_cannot_read_supervisor_files_or_parent_processes() {
    let (_root, workspace, state) = setup();
    let child = block_on(ContainedLocal::prepare(
        workspace.clone(),
        state.clone(),
        limits(),
    ))
    .unwrap();
    let literal = "literal; no shell expansion $(false)";
    let request = ExecRequest::new("/bin/sh").args([
        "-c",
        "test ! -e \"$1/secret\" && test ! -e \"/proc/$2/status\" && \
        test \"$HOME\" = /home/runner && test ! -e /var/run/docker.sock && \
        test ! -e /sys/fs/cgroup && test \"$(wc -l </proc/net/route)\" = 1 && \
        printf '%s' \"$3\" && printf 'created' > /workspace/marker",
        "contained",
        state.to_str().unwrap(),
        &std::process::id().to_string(),
        literal,
    ]);
    let result = block_on(block_on(child.exec(request)).unwrap().collect(4096));
    assert!(result.exit.success(), "{:?}", result.exit);
    assert_eq!(result.text(), literal);
    assert_eq!(fs::read(workspace.join("marker")).unwrap(), b"created");
    assert_eq!(fs::read(state.join("secret")).unwrap(), b"supervisor-only");
}

#[test]
#[ignore = "requires bubblewrap, prlimit and enabled Linux user namespaces"]
fn contained_pty_keeps_structured_execution_contained_and_revocable() {
    let (_root, workspace, state) = setup();
    let child = block_on(ContainedLocal::prepare(
        workspace.clone(),
        state.clone(),
        limits(),
    ))
    .unwrap();
    let source = ScriptedConnectionSource::default();
    let session = child.open_with(
        crate::tests::request(ShellLaunch::default()),
        ConnectionOpenOptions::new(source.source()),
    );
    let executor = session.exec().unwrap();
    let result = block_on(
        block_on(executor.exec(ExecRequest::new("/bin/sh").args([
            "-c",
            "test ! -e \"$1/secret\" && printf 'contained'",
            "check",
            state.to_str().unwrap(),
        ])))
        .unwrap()
        .collect(4096),
    );
    assert!(result.exit.success(), "{:?}", result.exit);
    assert_eq!(result.text(), "contained");
    assert!(session.input(b"printf 'pty-ready\\n'\n".to_vec()));
    let (send, receive) = std::sync::mpsc::channel();
    let observer = session.clone();
    let watch = std::thread::spawn(move || {
        block_on(async move {
            let mut bytes = Vec::new();
            while let Some(event) = observer.next_event().await {
                match event {
                    Event::Output(chunk) => {
                        bytes.extend(chunk);
                        if String::from_utf8_lossy(&bytes).contains("pty-ready\r\n") {
                            let _ = send.send(true);
                            return;
                        }
                    }
                    Event::Closed(_) => {
                        let _ = send.send(false);
                        return;
                    }
                    _ => {}
                }
            }
        })
    });
    assert!(receive.recv_timeout(Duration::from_secs(5)).unwrap());
    source.revoke();
    assert!(matches!(
        block_on(executor.exec(ExecRequest::new("/bin/true"))),
        Err(ExecError::Source(_))
    ));
    session.close();
    watch.join().unwrap();
}
