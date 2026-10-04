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

mod exec;

pub use exec::LocalExec;

/// A local shell factory, configured when a new terminal is opened.
#[derive(Clone, Debug)]
pub struct LocalTransport(pub ShellLaunch);

impl Transport for LocalTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        let (session, driver) = channel(None);
        let launch = self.0.clone();
        thread::spawn(move || {
            let result = start(launch, false, request, driver.clone());
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

/// A command PTY with the host environment and platform PTY metadata only.
/// Used for authentication commands whose credentials must not inherit ambient values.
#[derive(Clone, Debug)]
pub struct IsolatedLocalTransport(pub ShellLaunch);
impl Transport for IsolatedLocalTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        let (session, driver) = channel(None);
        let launch = self.0.clone();
        thread::spawn(move || {
            if let Err(error) = start(launch, true, request, driver.clone()) {
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
    isolated: bool,
    request: ConnectRequest,
    driver: nocterm_session::SessionDriver,
) -> Result<(), String> {
    launch.validate()?;
    let pair = native_pty_system()
        .openpty(size(request.size))
        .map_err(|e| e.to_string())?;
    let program = launch.program.clone().unwrap_or_else(default_shell);
    let mut command = CommandBuilder::new(&program);
    if isolated {
        command.env_clear();
    }
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
mod tests;
