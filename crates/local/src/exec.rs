//! Programs run on this computer, with process lifetime owned by a worker.

use std::{
    io::{Read, Write},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use futures::{FutureExt, channel::oneshot, executor::block_on, select};
use nocterm_session::{
    ErrorTail, ExecError, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec, Session,
    ShellLaunch, TerminalRequest,
};

mod admission;
mod process;
use admission::{Admission, Permit};
#[cfg(all(test, unix))]
mod tests;

const READ_BUFFER: usize = 16 * 1024;
#[cfg(not(unix))]
const EXIT_POLL: Duration = Duration::from_millis(100);
const ERROR_GRACE: Duration = Duration::from_millis(200);

/// One explicitly shared native-command admission owner.
#[derive(Clone, Debug, Default)]
pub struct LocalExec {
    admission: Arc<Admission>,
}

pub(crate) struct ScopedLocalExec {
    source: nocterm_session::ConnectionSource,
    executor: LocalExec,
}
impl ScopedLocalExec {
    pub(crate) fn new(source: nocterm_session::ConnectionSource) -> Self {
        Self {
            source,
            executor: LocalExec::default(),
        }
    }
}
impl HostExec for LocalExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        execute(request, Default::default(), self.admission.clone())
    }
    fn terminal(&self, request: TerminalRequest) -> Session {
        terminal(request, Default::default())
    }
}
impl HostExec for ScopedLocalExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        execute(
            request,
            self.source.clone(),
            self.executor.admission.clone(),
        )
    }
    fn terminal(&self, request: TerminalRequest) -> Session {
        terminal(request, self.source.clone())
    }
}
pub(crate) fn execute_scoped(
    request: ExecRequest,
    source: nocterm_session::ConnectionSource,
    executor: &LocalExec,
) -> ExecFuture<ExecOutput> {
    execute(request, source, executor.admission.clone())
}
fn execute(
    request: ExecRequest,
    source: nocterm_session::ConnectionSource,
    admission: Arc<Admission>,
) -> ExecFuture<ExecOutput> {
    source.clone().guard(async move {
        let permit = admission.reserve()?;
        let (ready, started) = oneshot::channel();
        spawn("nocterm-exec-start", move || {
            if !ready.is_canceled() {
                let _ = ready.send(start(request, source, permit));
            }
        })?;
        started
            .await
            .map_err(|error| ExecError::Failed(error.to_string()))?
    })
}
fn terminal(request: TerminalRequest, source: nocterm_session::ConnectionSource) -> Session {
    let launch = ShellLaunch {
        program: Some(request.program.program),
        args: request.program.args,
        integration: false,
        ..ShellLaunch::default()
    };
    crate::open_scoped(launch, false, request.term, request.size, source)
}

fn start(
    request: ExecRequest,
    source: nocterm_session::ConnectionSource,
    permit: Arc<Permit>,
) -> Result<ExecOutput, ExecError> {
    source.check()?;
    let mut process = process::Process::spawn(&request, &source)?;
    let (Some(stdout), Some(stderr)) = (process.stdout(), process.stderr()) else {
        return Err(ExecError::Failed("no output pipe".into()));
    };
    let stopped = Arc::new(AtomicBool::new(false));
    if let (Some(input), Some(stdin)) = (request.stdin, process.stdin()) {
        write_input(
            stdin,
            input,
            source.clone(),
            stopped.clone(),
            permit.clone(),
        )
        .inspect_err(|_| stopped.store(true, Ordering::Release))?;
    }
    let (sink, output) = ExecOutput::channel_with_source(source.clone());
    let sink = Arc::new(sink);
    let drained = read_output(stdout, sink.clone(), stopped.clone(), permit.clone())
        .inspect_err(|_| stopped.store(true, Ordering::Release))?;
    let errors = read_errors(stderr, stopped.clone(), permit.clone())
        .inspect_err(|_| stopped.store(true, Ordering::Release))?;
    let failed = stopped.clone();
    spawn("nocterm-exec-wait", move || {
        // Observe the leader independently from pipe EOF: descendants may
        // retain both output pipes after the leader has exited.
        let _permit = permit;
        let mut pause = Duration::from_millis(1);
        let natural = loop {
            if sink.is_closed() || source.check().is_err() {
                break false;
            }
            match process.exited() {
                Ok(true) => break true,
                Ok(false) => {
                    thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(100));
                }
                Err(_) => break false,
            }
        };
        let status = process.stop().filter(|_| natural);
        stopped.store(true, Ordering::Release);
        block_on(async {
            select! {
                _ = drained.fuse() => (),
                () = sink.closed().fuse() => (),
            }
        });
        let tail = errors.recv_timeout(ERROR_GRACE).unwrap_or_default();
        sink.finish(tail.exit(status));
    })
    .inspect_err(|_| failed.store(true, Ordering::Release))?;
    Ok(output)
}

fn write_input(
    mut stdin: ChildStdin,
    input: Vec<u8>,
    source: nocterm_session::ConnectionSource,
    stopped: Arc<AtomicBool>,
    permit: Arc<Permit>,
) -> Result<(), ExecError> {
    #[cfg(unix)]
    process::nonblocking(&stdin)?;
    spawn("nocterm-exec-in", move || {
        let _permit = permit;
        let mut remaining = input.as_slice();
        while !remaining.is_empty() && source.check().is_ok() && !stopped.load(Ordering::Acquire) {
            match stdin.write(&remaining[..remaining.len().min(1024)]) {
                Ok(0) => break,
                Ok(n) => remaining = &remaining[n..],
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    wait_writable(&stdin)
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    })
}

fn read_output(
    mut stdout: ChildStdout,
    sink: Arc<ExecSink>,
    stopped: Arc<AtomicBool>,
    permit: Arc<Permit>,
) -> Result<oneshot::Receiver<()>, ExecError> {
    #[cfg(unix)]
    process::nonblocking(&stdout)?;
    let (finished, drained) = oneshot::channel();
    spawn("nocterm-exec-out", move || {
        let _permit = permit;
        let mut buffer = vec![0; READ_BUFFER];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if !block_on(sink.send(buffer[..read].to_vec())) {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopped.load(Ordering::Acquire) || sink.is_closed() {
                        break;
                    }
                    wait_readable(&stdout);
                }
                Err(_) => break,
            }
        }
        drop(finished);
    })?;
    Ok(drained)
}

fn read_errors(
    mut stderr: ChildStderr,
    stopped: Arc<AtomicBool>,
    permit: Arc<Permit>,
) -> Result<std::sync::mpsc::Receiver<ErrorTail>, ExecError> {
    #[cfg(unix)]
    process::nonblocking(&stderr)?;
    let (send, errors) = std::sync::mpsc::channel();
    spawn("nocterm-exec-err", move || {
        let _permit = permit;
        let mut tail = ErrorTail::default();
        let mut buffer = [0; 1024];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => tail.push(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    wait_readable(&stderr);
                }
                Err(_) => break,
            }
        }
        let _ = send.send(tail);
    })?;
    Ok(errors)
}

fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> Result<(), ExecError> {
    thread::Builder::new()
        .name(name.into())
        .spawn(body)
        .map(drop)
        .map_err(|error| ExecError::Failed(error.to_string()))
}

#[cfg(unix)]
fn wait_readable(pipe: &impl std::os::fd::AsFd) {
    wait_pipe(pipe, nix::poll::PollFlags::POLLIN);
}
#[cfg(unix)]
fn wait_writable(pipe: &impl std::os::fd::AsFd) {
    wait_pipe(pipe, nix::poll::PollFlags::POLLOUT);
}
#[cfg(unix)]
fn wait_pipe(pipe: &impl std::os::fd::AsFd, events: nix::poll::PollFlags) {
    let mut fds = [nix::poll::PollFd::new(pipe.as_fd(), events)];
    let _ = nix::poll::poll(&mut fds, 100u16);
}
#[cfg(not(unix))]
fn wait_readable<T>(_: &T) {
    thread::sleep(EXIT_POLL);
}
#[cfg(not(unix))]
fn wait_writable<T>(_: &T) {
    thread::sleep(EXIT_POLL);
}
