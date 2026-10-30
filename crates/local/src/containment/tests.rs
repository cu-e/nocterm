//! Actual namespace, host-exec and PTY behavior, without a synthetic launcher.
use super::*;
use futures::executor::block_on;
use nocterm_session::{Event, testing::ScriptedConnectionSource};
use std::{fs, os::unix::fs::PermissionsExt as _};

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
    let events = crate::tests::events(&session);
    let initial = prompt(&events);
    assert!(initial.contains("\x1b]7;file://localhost/workspace\x07"));
    assert!(session.input(b"test ! -w /run/nocterm/bashrc && test \"$HOME\" = /home/runner && printf 'pty-ready|%s|%s|%s\\n' \"$TERM\" \"$PWD\" '$(false); literal'\r".to_vec()));
    let output = prompt(&events);
    assert!(output.contains("\x1b]133;C\x07"), "{output:?}");
    assert!(
        output.contains("pty-ready|xterm-256color|/workspace|$(false); literal\r\n"),
        "{output:?}"
    );
    source.revoke();
    assert!(matches!(
        block_on(executor.exec(ExecRequest::new("/bin/true"))),
        Err(ExecError::Source(_))
    ));
    session.close();
    while !matches!(crate::tests::next(&events), Event::Closed(_)) {}
}

fn prompt(events: &std::sync::mpsc::Receiver<Event>) -> String {
    let mut output = Vec::new();
    while !output.windows(8).any(|bytes| bytes == b"\x1b]133;B\x07") {
        match crate::tests::next(events) {
            Event::Output(bytes) => output.extend(bytes),
            Event::Closed(reason) => panic!("closed before a real shell prompt: {reason:?}"),
            _ => {}
        }
        assert!(output.len() <= 32 * 1024, "unexpected output before prompt");
    }
    String::from_utf8(output).unwrap()
}

#[test]
#[ignore = "requires bubblewrap, prlimit and enabled Linux user namespaces"]
fn contained_terminal_keeps_literal_commands_and_refuses_unsupported_shell_settings() {
    let (_root, workspace, state) = setup();
    let child = block_on(ContainedLocal::prepare(workspace, state, limits())).unwrap();
    let session = child.terminal(TerminalRequest {
        program: ExecRequest::new("/bin/sh").args([
            "-c",
            "printf '%s|%s' \"$TERM\" \"$1\"",
            "literal",
            "$(false); literal",
        ]),
        term: "xterm-256color".into(),
        size: Default::default(),
    });
    let events = crate::tests::events(&session);
    let mut output = Vec::new();
    loop {
        match crate::tests::next(&events) {
            Event::Output(bytes) => output.extend(bytes),
            Event::Closed(reason) => {
                assert_eq!(reason, nocterm_session::CloseReason::Exited(Some(0)));
                break;
            }
            _ => {}
        }
    }
    assert_eq!(output, b"xterm-256color|$(false); literal");
    let unsupported = [
        ShellLaunch {
            program: Some("/bin/sh".into()),
            ..Default::default()
        },
        ShellLaunch {
            args: vec!["-c".into(), "exit".into()],
            ..Default::default()
        },
        ShellLaunch {
            cwd: Some("/home/runner".into()),
            ..Default::default()
        },
        ShellLaunch {
            env: [("HOME".into(), "/host".into())].into(),
            ..Default::default()
        },
    ];
    for launch in unsupported {
        let refused = child.open(crate::tests::request(launch));
        assert!(matches!(
            block_on(refused.next_event()),
            Some(Event::Closed(nocterm_session::CloseReason::Failed(_)))
        ));
    }
    let mut invalid_term = crate::tests::request(ShellLaunch::default());
    invalid_term.term = "xterm\nsetenv".into();
    assert!(matches!(
        block_on(child.open(invalid_term).next_event()),
        Some(Event::Closed(nocterm_session::CloseReason::Failed(_)))
    ));
}
