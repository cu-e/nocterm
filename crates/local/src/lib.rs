//! Local PTY adapter. Bounded I/O workers and an independent close observer
//! ensure process cleanup never waits behind a blocked PTY write.

use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use futures::{FutureExt, executor::block_on, select};
use nocterm_session::{
    CloseReason, Command, ConnectRequest, Event, PtySize, Session, SessionError, ShellLaunch,
    Transport, channel,
};
use portable_pty::{CommandBuilder, PtySize as NativeSize, native_pty_system};

/// A local shell factory, configured when a new terminal is opened.
#[derive(Clone, Debug)]
pub struct LocalTransport(pub ShellLaunch);

impl Transport for LocalTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        let (session, driver) = channel(None);
        let launch = self.0.clone();
        thread::spawn(move || {
            let result = start(launch, request, driver.clone());
            if let Err(error) = result {
                block_on(
                    driver.emit(Event::Closed(CloseReason::Failed(SessionError::Other(
                        error,
                    )))),
                );
            }
        });
        session
    }
}

fn size(s: PtySize) -> NativeSize {
    NativeSize {
        rows: s.rows,
        cols: s.cols,
        pixel_width: s.pixel_width,
        pixel_height: s.pixel_height,
    }
}

fn start(
    launch: ShellLaunch,
    request: ConnectRequest,
    driver: nocterm_session::SessionDriver,
) -> Result<(), String> {
    launch.validate()?;
    let pair = native_pty_system()
        .openpty(size(request.size))
        .map_err(|e| e.to_string())?;
    let program = launch.program.clone().unwrap_or_else(default_shell);
    let mut command = CommandBuilder::new(&program);
    let integration = if launch.integration && launch.args.is_empty() {
        prepare_integration(&program, &mut command)?
    } else {
        None
    };
    if integration.is_none() {
        command.args(&launch.args);
    }
    if let Some(cwd) = launch.cwd.clone().or_else(home) {
        command.cwd(cwd);
    }
    command.env("TERM", request.term);
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    #[cfg(unix)]
    nonblocking(pair.master.as_ref())?;
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let child_pid = child.process_id();
    drop(pair.slave);
    let master = Arc::new(Mutex::new(pair.master));
    let mut killer = child.clone_killer();
    let mut close_killer = child.clone_killer();
    #[cfg(unix)]
    let shell_group = child_pid.map(|pid| pid as i32);
    #[cfg(not(unix))]
    let shell_group = None;
    let (done_tx, done_rx) = async_channel::bounded::<()>(1);
    let command_driver = driver.clone();
    let closed_by_user = Arc::new(AtomicBool::new(false));
    let closing = closed_by_user.clone();
    let close_driver = driver.clone();
    let close_done = done_rx.clone();
    let close_flag = closed_by_user.clone();
    let close_master = master.clone();
    let command_master = master.clone();
    thread::spawn(move || {
        let requested = block_on(async {
            select! {
                _ = close_driver.closed().fuse() => true,
                _ = close_done.recv().fuse() => false,
            }
        });
        if requested {
            close_flag.store(true, Ordering::Release);
            kill_process(close_killer.as_mut(), &close_master, shell_group);
        }
    });
    thread::spawn(move || {
        loop {
            let (finished, next) = block_on(async {
                select! {
                    command = command_driver.next_command().fuse() => (false, command),
                    _ = done_rx.recv().fuse() => (true, None),
                }
            });
            if finished {
                break;
            }
            match next {
                Some(Command::Input(bytes)) => {
                    if write_input(writer.as_mut(), &bytes, &command_driver, &done_rx).is_err() {
                        if !done_rx.is_closed() {
                            kill_process(killer.as_mut(), &command_master, shell_group);
                        }
                        break;
                    }
                }
                Some(Command::Resize(s)) => {
                    let _ = command_master
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .resize(size(s));
                }
                Some(Command::Close) | None => {
                    closing.store(true, Ordering::Release);
                    kill_process(killer.as_mut(), &command_master, shell_group);
                    break;
                }
            }
        }
    });
    if !block_on(driver.emit(Event::Connected)) {
        let mut disconnected_killer = child.clone_killer();
        kill_process(disconnected_killer.as_mut(), &master, shell_group);
    }
    let mut buffer = [0; 8192];
    loop {
        // Closing is independent of slave EOF: a disowned/HUP-ignoring
        // background job can legitimately retain the terminal descriptor.
        if driver.closed().now_or_never().is_some() {
            closed_by_user.store(true, Ordering::Release);
            let mut read_killer = child.clone_killer();
            kill_process(read_killer.as_mut(), &master, shell_group);
            break;
        }
        match reader.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // The owned shell can also exit normally while a background
                // job retains the slave; stop after its buffered output drains.
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    done_tx.close();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let delivered = block_on(async {
                    select! {
                        emitted = driver.emit(Event::Output(buffer[..count].to_vec())).fuse() => emitted,
                        _ = driver.closed().fuse() => {
                            closed_by_user.store(true, Ordering::Release);
                            false
                        },
                    }
                });
                if !delivered {
                    let mut output_killer = child.clone_killer();
                    kill_process(output_killer.as_mut(), &master, shell_group);
                    break;
                }
            }
        }
    }
    let exit = child.wait().map_err(|e| e.to_string())?;
    done_tx.close();
    // Keep ephemeral scripts alive until the shell has exited.
    drop(integration);
    block_on(
        driver.emit(Event::Closed(if closed_by_user.load(Ordering::Acquire) {
            CloseReason::ClosedByUser
        } else {
            CloseReason::Exited(Some(exit.exit_code()))
        })),
    );
    Ok(())
}

#[cfg(unix)]
fn nonblocking(master: &dyn portable_pty::MasterPty) -> Result<(), String> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let fd = master
        .as_raw_fd()
        .ok_or("native PTY has no file descriptor")?;
    let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(|e| e.to_string())?;
    fcntl(
        fd,
        FcntlArg::F_SETFL(OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn write_input(
    writer: &mut dyn Write,
    bytes: &[u8],
    driver: &nocterm_session::SessionDriver,
    done: &async_channel::Receiver<()>,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        if done.is_closed() || driver.closed().now_or_never().is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "PTY closed",
            ));
        }
        match writer.write(&bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(count) => offset += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    loop {
        if done.is_closed() || driver.closed().now_or_never().is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "PTY closed",
            ));
        }
        match writer.flush() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

fn kill_process(
    killer: &mut dyn portable_pty::ChildKiller,
    master: &Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    shell_group: Option<i32>,
) {
    // Job control changes the foreground group after launch. Query the live
    // terminal at close; this short metadata lock never surrounds PTY writes.
    #[cfg(unix)]
    let foreground = master
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .process_group_leader();
    #[cfg(unix)]
    let process_groups = [foreground, shell_group];
    #[cfg(unix)]
    for group in process_groups.into_iter().flatten() {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = (master, shell_group);
    let _ = killer.kill();
}

fn home() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

/// The OS user's configured shell, with a conventional platform fallback.
pub fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

fn prepare_integration(
    program: &str,
    command: &mut CommandBuilder,
) -> Result<Option<tempfile::TempDir>, String> {
    let stem = Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let (name, script) = match stem {
        "bash" => (
            "bashrc",
            include_str!("../../../assets/shell-integration/bash.bash"),
        ),
        "zsh" => (
            ".zshrc",
            include_str!("../../../assets/shell-integration/zsh.zsh"),
        ),
        "fish" => (
            "integration.fish",
            include_str!("../../../assets/shell-integration/fish.fish"),
        ),
        "pwsh" | "powershell" => (
            "integration.ps1",
            include_str!("../../../assets/shell-integration/powershell.ps1"),
        ),
        _ => return Ok(None),
    };
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join(name);
    std::fs::write(&path, script).map_err(|e| e.to_string())?;
    match stem {
        "bash" => {
            command.args(["--rcfile", &path.to_string_lossy(), "-i"]);
        }
        "zsh" => {
            std::fs::write(directory.path().join(".zshenv"),
                "__nocterm_temp_zdotdir=$ZDOTDIR\nif [[ -n $NOCTERM_ORIGINAL_ZDOTDIR ]]; then ZDOTDIR=$NOCTERM_ORIGINAL_ZDOTDIR; else unset ZDOTDIR; fi\n[[ -f ${ZDOTDIR:-$HOME}/.zshenv ]] && source \"${ZDOTDIR:-$HOME}/.zshenv\"\nNOCTERM_ORIGINAL_ZDOTDIR=${ZDOTDIR:-}\nZDOTDIR=$__nocterm_temp_zdotdir\n")
                .map_err(|e|e.to_string())?;
            command.env(
                "NOCTERM_ORIGINAL_ZDOTDIR",
                std::env::var("ZDOTDIR").unwrap_or_default(),
            );
            command.env("ZDOTDIR", directory.path());
            command.arg("-i");
        }
        "fish" => {
            command.args([
                "-i",
                "-C",
                &format!(
                    "source {}",
                    nocterm_session::quote_posix(&path.to_string_lossy())
                ),
            ]);
        }
        _ => {
            command.args([
                "-NoExit",
                "-Command",
                &format!(
                    ". {}",
                    nocterm_session::quote_powershell(&path.to_string_lossy())
                ),
            ]);
        }
    }
    Ok(Some(directory))
}

#[cfg(test)]
mod tests {
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
                String::from_utf8_lossy(&out)
                    .contains(&format!("file://localhost{}", directory.path().display())),
                "{executable}: {}",
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
            let stat =
                std::fs::read_to_string(format!("/proc/{descendant}/stat")).unwrap_or_default();
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
            let stat =
                std::fs::read_to_string(format!("/proc/{foreground}/stat")).unwrap_or_default();
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
}
