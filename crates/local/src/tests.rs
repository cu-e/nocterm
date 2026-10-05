use super::*;
fn request(launch: ShellLaunch) -> ConnectRequest {
    ConnectRequest {
        launch,
        target: nocterm_session::Target::new("user", "localhost", 22),
        auth: nocterm_session::Auth::Auto,
        term: "xterm-256color".into(),
        size: Default::default(),
        connect_timeout: std::time::Duration::from_secs(5),
        keepalive_interval: None,
        proxy: Default::default(),
    }
}
fn events(session: &Session) -> std::sync::mpsc::Receiver<Event> {
    let session = session.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        block_on(async move {
            while let Some(event) = session.next_event().await {
                let closed = matches!(event, Event::Closed(_));
                if tx.send(event).is_err() || closed {
                    break;
                }
            }
        })
    });
    rx
}
fn next(rx: &std::sync::mpsc::Receiver<Event>) -> Event {
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .expect("PTY event timed out")
}
#[cfg(unix)]
#[test]
fn isolated_command_has_only_explicit_environment() {
    let launch = ShellLaunch {
        program: Some("/usr/bin/env".into()),
        env: [
            ("AUTH_TEST_VALUE".into(), "literal value".into()),
            ("SHELL".into(), "/bin/sh".into()),
        ]
        .into(),
        integration: false,
        ..Default::default()
    };
    let session = IsolatedLocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut output = Vec::new();
    loop {
        match next(&rx) {
            Event::Output(bytes) => output.extend(bytes),
            Event::Closed(reason) => {
                assert_eq!(reason, CloseReason::Exited(Some(0)));
                break;
            }
            _ => {}
        }
    }
    let output = String::from_utf8(output).unwrap();
    let mut lines = output.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![
            "AUTH_TEST_VALUE=literal value",
            "SHELL=/bin/sh",
            "TERM=xterm-256color"
        ]
    );
}

#[cfg(unix)]
#[test]
fn isolated_command_preserves_argv_and_nonzero_exit() {
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec![
            "-c".into(),
            "printf '%s' \"$1\"; exit 7".into(),
            "auth".into(),
            "$(printf injected); spaces & quotes'".into(),
        ],
        integration: false,
        ..Default::default()
    };
    let session = IsolatedLocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut output = Vec::new();
    loop {
        match next(&rx) {
            Event::Output(bytes) => output.extend(bytes),
            Event::Closed(reason) => {
                assert_eq!(reason, CloseReason::Exited(Some(7)));
                break;
            }
            _ => {}
        }
    }
    assert_eq!(output, b"$(printf injected); spaces & quotes'");
}

#[cfg(unix)]
#[test]
fn isolated_command_user_close_reaps_process() {
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "printf auth-ready; exec sleep 30".into()],
        env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
        integration: false,
        ..Default::default()
    };
    let session = IsolatedLocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    while !matches!(next(&rx), Event::Output(_)) {}
    session.close();
    loop {
        if let Event::Closed(reason) = next(&rx) {
            assert_eq!(reason, CloseReason::ClosedByUser);
            break;
        }
    }
}

#[cfg(unix)]
#[test]
fn local_pty_emits_output_and_reaps_exit() {
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "printf local-pty-ok; exit 7".into()],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut output = Vec::new();
    loop {
        match next(&rx) {
            Event::Output(bytes) => output.extend(bytes),
            Event::Closed(CloseReason::Exited(Some(7))) => break,
            Event::Closed(other) => panic!("{other:?}"),
            _ => {}
        }
    }
    assert!(String::from_utf8_lossy(&output).contains("local-pty-ok"));
}
#[cfg(unix)]
#[test]
fn launch_cwd_environment_and_resize_use_real_pty() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().join("space ' directory");
    std::fs::create_dir(&cwd).unwrap();
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec![
            "-c".into(),
            "pwd; printf '%s' \"$NOCTERM_PTY_TEST\"; stty size".into(),
        ],
        cwd: Some(cwd.to_string_lossy().into_owned()),
        env: std::collections::BTreeMap::from([("NOCTERM_PTY_TEST".into(), "env-ok ".into())]),
        integration: false,
    };
    let mut req = request(launch.clone());
    req.size.cols = 97;
    req.size.rows = 31;
    let session = LocalTransport(launch).open(req);
    let rx = events(&session);
    let mut out = Vec::new();
    loop {
        match next(&rx) {
            Event::Output(bytes) => out.extend(bytes),
            Event::Closed(CloseReason::Exited(Some(0))) => break,
            Event::Closed(e) => panic!("{e:?}"),
            _ => {}
        }
    }
    let output = String::from_utf8_lossy(&out);
    assert!(output.contains(cwd.to_str().unwrap()));
    assert!(output.contains("env-ok 31 97"), "{output}");
}
#[cfg(unix)]
#[test]
fn close_terminates_and_reaps_an_interactive_process() {
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "echo pid=$$; exec sleep 60".into()],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut out = Vec::new();
    while !out.contains(&b'\n') {
        if let Event::Output(bytes) = next(&rx) {
            out.extend(bytes);
        }
    }
    let output = String::from_utf8_lossy(&out);
    let pid = output
        .trim()
        .strip_prefix("pid=")
        .unwrap()
        .parse::<i32>()
        .unwrap();
    session.close();
    loop {
        if let Event::Closed(reason) = next(&rx) {
            assert_eq!(reason, CloseReason::ClosedByUser);
            break;
        }
    }
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
}
#[cfg(unix)]
#[test]
fn bash_integration_reports_prompt_cwd_and_execution() {
    let directory = tempfile::tempdir().unwrap();
    let launch = ShellLaunch {
        program: Some("/bin/bash".into()),
        cwd: Some(directory.path().to_string_lossy().into_owned()),
        env: std::collections::BTreeMap::from([(
            "HOME".into(),
            directory.path().to_string_lossy().into_owned(),
        )]),
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut out = Vec::new();
    while !out.windows(6).any(|w| w == b"133;B\x07") {
        if let Event::Output(bytes) = next(&rx) {
            out.extend(bytes);
        }
    }
    assert!(String::from_utf8_lossy(&out).contains("7;file://localhost"));
    session.input(b"printf integration-ok\r".to_vec());
    out.clear();
    while !String::from_utf8_lossy(&out).contains("integration-ok\x1b]7;") {
        if let Event::Output(bytes) = next(&rx) {
            out.extend(bytes);
        }
    }
    assert!(out.windows(6).any(|w| w == b"133;C\x07"));
    session.close();
    loop {
        if matches!(next(&rx), Event::Closed(_)) {
            break;
        }
    }
}

#[cfg(unix)]
#[test]
fn zsh_and_fish_integration_keep_cwd_and_prompt_markers() {
    for executable in ["/usr/bin/zsh", "/usr/bin/fish"] {
        if !Path::new(executable).exists() {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let launch = ShellLaunch {
            program: Some(executable.into()),
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            env: std::collections::BTreeMap::from([(
                "HOME".into(),
                directory.path().to_string_lossy().into_owned(),
            )]),
            ..Default::default()
        };
        let session = LocalTransport(launch.clone()).open(request(launch));
        let rx = events(&session);
        let mut out = Vec::new();
        while !out.windows(6).any(|w| w == b"133;A\x07") {
            match next(&rx) {
                Event::Output(bytes) => out.extend(bytes),
                Event::Closed(e) => {
                    panic!("{executable}: {e:?}: {}", String::from_utf8_lossy(&out))
                }
                _ => {}
            }
        }
        assert!(
            String::from_utf8_lossy(&out).contains(&format!(
                "file://localhost{}\x07",
                directory.path().display()
            )),
            "{executable} must report the directory exactly: {}",
            String::from_utf8_lossy(&out)
        );
        session.input(b"printf 'integration-zf-ok\\n'\r".to_vec());
        out.clear();
        while !out.windows(6).any(|w| w == b"133;C\x07") {
            if let Event::Output(bytes) = next(&rx) {
                out.extend(bytes);
            }
        }
        session.close();
        loop {
            if matches!(next(&rx), Event::Closed(_)) {
                break;
            }
        }
    }
}
#[cfg(unix)]
#[test]
fn close_interrupts_full_input_and_kills_shell_process_group() {
    use std::time::{Duration, Instant};
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "stty -echo -icanon; sleep 30 & worker=$!; printf 'ready=%s child=%s\\n' \"$$\" \"$worker\"; wait".into()],
        integration: false, ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut bytes = Vec::new();
    while !bytes.contains(&b'\n') {
        if let Event::Output(chunk) = next(&rx) {
            bytes.extend(chunk);
        }
    }
    let output = String::from_utf8_lossy(&bytes);
    let mut fields = output.split_whitespace();
    let parent: i32 = fields
        .next()
        .unwrap()
        .strip_prefix("ready=")
        .unwrap()
        .parse()
        .unwrap();
    let descendant: i32 = fields
        .next()
        .unwrap()
        .strip_prefix("child=")
        .unwrap()
        .parse()
        .unwrap();
    let mut rejected = false;
    for _ in 0..128 {
        rejected |= !session.input(vec![b'x'; 64 * 1024]);
    }
    assert!(rejected, "input queue must be bounded");
    thread::sleep(Duration::from_millis(100));
    let deadline = Instant::now() + Duration::from_secs(2);
    session.close();
    assert!(!session.input(b"closed".to_vec()));
    loop {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("close was blocked behind PTY write");
        if let Event::Closed(reason) = event {
            assert_eq!(reason, CloseReason::ClosedByUser);
            break;
        }
    }
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(parent), None),
        Err(nix::errno::Errno::ESRCH)
    );
    // The direct child is reaped by this adapter. A killed grandchild is
    // adopted/reaped by init; Linux containers can leave an init-owned
    // zombie briefly, but no descendant may remain executing.
    let descendant_alive =
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(descendant), None).is_ok();
    #[cfg(target_os = "linux")]
    if descendant_alive {
        let stat = std::fs::read_to_string(format!("/proc/{descendant}/stat")).unwrap_or_default();
        assert!(
            stat.is_empty()
                || stat
                    .rsplit_once(") ")
                    .is_some_and(|(_, fields)| fields.starts_with('Z')),
            "descendant still running: {stat}"
        );
    }
    #[cfg(not(target_os = "linux"))]
    assert!(!descendant_alive);
}

#[cfg(unix)]
#[test]
fn close_interrupts_output_backpressure_before_listener_drains_events() {
    use std::time::{Duration, Instant};
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "exec yes".into()],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    thread::sleep(Duration::from_millis(150));
    let deadline = Instant::now() + Duration::from_secs(2);
    session.close();
    let rx = events(&session);
    loop {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("close was blocked behind output backpressure");
        if let Event::Closed(reason) = event {
            assert_eq!(reason, CloseReason::ClosedByUser);
            break;
        }
    }
}
#[cfg(target_os = "linux")]
#[test]
fn close_kills_current_foreground_job_even_when_it_ignores_hangup() {
    use std::time::{Duration, Instant};
    let launch = ShellLaunch {
        program: Some("/bin/bash".into()),
        args: vec![
            "--noprofile".into(),
            "--norc".into(),
            "-ic".into(),
            "trap '' HUP; sleep 30 & job=$!; echo review-shell=$$ review-child=$job; fg".into(),
        ],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut output = Vec::new();
    let (parent, foreground) = loop {
        if let Event::Output(bytes) = next(&rx) {
            output.extend(bytes);
        }
        let text = String::from_utf8_lossy(&output);
        if let Some(line) = text
            .lines()
            .find(|line| line.starts_with("review-shell=") && line.contains("review-child="))
        {
            if !output.ends_with(b"\n") {
                continue;
            }
            let mut fields = line.split_whitespace();
            let parent = fields
                .next()
                .unwrap()
                .strip_prefix("review-shell=")
                .unwrap()
                .parse::<i32>()
                .unwrap();
            let foreground = fields
                .next()
                .unwrap()
                .strip_prefix("review-child=")
                .unwrap()
                .parse::<i32>()
                .unwrap();
            break (parent, foreground);
        }
    };
    // Wait for Bash's real tcsetpgrp, rather than assuming that the ready
    // line and the foreground handoff happen in one scheduler turn.
    let ready_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{parent}/stat")).unwrap();
        let current = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(5)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        if current == foreground {
            break;
        }
        if Instant::now() >= ready_deadline {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(foreground),
                nix::sys::signal::Signal::SIGKILL,
            );
            session.close();
            panic!("Bash never gave its job the terminal: {stat}");
        }
        thread::sleep(Duration::from_millis(5));
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    session.close();
    loop {
        let event = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        if event.is_err() {
            // Rescue a failing implementation so it cannot retain the PTY
            // and an extra sleep process for the rest of the test suite.
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(foreground),
                nix::sys::signal::Signal::SIGKILL,
            );
            let _ = rx.recv_timeout(Duration::from_secs(1));
            panic!("Close left the foreground job alive holding the PTY");
        }
        if let Event::Closed(reason) = event.unwrap() {
            assert_eq!(reason, CloseReason::ClosedByUser);
            break;
        }
    }
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(parent), None),
        Err(nix::errno::Errno::ESRCH)
    );
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{foreground}/stat")).unwrap_or_default();
        if stat.is_empty()
            || stat
                .rsplit_once(") ")
                .is_some_and(|(_, fields)| fields.starts_with('Z'))
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(foreground),
                nix::sys::signal::Signal::SIGKILL,
            );
            panic!("foreground job still executing after close: {stat}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(target_os = "linux")]
#[test]
fn background_hangup_ignoring_job_cannot_block_close() {
    use std::time::{Duration, Instant};
    let launch = ShellLaunch {
        program: Some("/bin/bash".into()),
        args: vec![
            "--noprofile".into(),
            "--norc".into(),
            "-ic".into(),
            "trap '' HUP; sleep 30 & job=$!; echo bg-shell=$$ bg-child=$job; wait".into(),
        ],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut out = Vec::new();
    let (parent, job) = loop {
        if let Event::Output(bytes) = next(&rx) {
            out.extend(bytes);
        }
        let text = String::from_utf8_lossy(&out);
        if !out.ends_with(b"\n") {
            continue;
        }
        if let Some(line) = text.lines().find(|line| line.starts_with("bg-shell=")) {
            let mut fields = line.split_whitespace();
            break (
                fields
                    .next()
                    .unwrap()
                    .strip_prefix("bg-shell=")
                    .unwrap()
                    .parse::<i32>()
                    .unwrap(),
                fields
                    .next()
                    .unwrap()
                    .strip_prefix("bg-child=")
                    .unwrap()
                    .parse::<i32>()
                    .unwrap(),
            );
        }
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    session.close();
    let closed = loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Event::Closed(reason)) => break Some(reason),
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    // This intentionally detached job belongs to the test. The adapter
    // must close independently of its slave FD; it does not enumerate and
    // kill arbitrary detached/user daemon processes.
    let _ = nix::sys::signal::killpg(
        nix::unistd::Pid::from_raw(job),
        nix::sys::signal::Signal::SIGKILL,
    );
    assert_eq!(
        closed,
        Some(CloseReason::ClosedByUser),
        "background slave FD blocked close"
    );
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(parent), None),
        Err(nix::errno::Errno::ESRCH)
    );
}

#[cfg(unix)]
#[test]
fn large_input_survives_nonblocking_backpressure_without_partial_writes() {
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec![
            "-c".into(),
            "stty raw -echo; printf ready-input; sleep 0.2; head -c 500000 | wc -c".into(),
        ],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let rx = events(&session);
    let mut out = Vec::new();
    while !String::from_utf8_lossy(&out).contains("ready-input") {
        if let Event::Output(bytes) = next(&rx) {
            out.extend(bytes);
        }
    }
    assert!(session.input(vec![b'x'; 500_000]));
    out.clear();
    loop {
        match next(&rx) {
            Event::Output(bytes) => out.extend(bytes),
            Event::Closed(reason) => {
                assert_eq!(reason, CloseReason::Exited(Some(0)));
                break;
            }
            _ => {}
        }
    }
    assert!(
        String::from_utf8_lossy(&out).contains("500000"),
        "lost/repeated input bytes: {}",
        String::from_utf8_lossy(&out)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn dropping_the_last_session_owner_reaps_shell_without_event_consumer() {
    use std::time::{Duration, Instant};
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "echo drop-owner=$$; exec sleep 30".into()],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let (send, receive) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut out = Vec::new();
        loop {
            match block_on(session.next_event()) {
                Some(Event::Output(bytes)) => out.extend(bytes),
                Some(Event::Closed(reason)) => panic!("shell exited before ready: {reason:?}"),
                _ => {}
            }
            if !out.ends_with(b"\n") {
                continue;
            }
            let text = String::from_utf8_lossy(&out);
            if let Some(line) = text.lines().find(|line| line.starts_with("drop-owner=")) {
                let pid = line
                    .strip_prefix("drop-owner=")
                    .unwrap()
                    .trim()
                    .parse::<i32>()
                    .unwrap();
                send.send(pid).unwrap();
                drop(session);
                break;
            }
        }
    });
    let pid = receive.recv_timeout(Duration::from_secs(2)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
        if Instant::now() >= deadline {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
            panic!("dropping the last owner left the shell unreaped");
        }
        thread::sleep(Duration::from_millis(5));
    }
}
