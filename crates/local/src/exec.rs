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

mod process;
#[cfg(all(test, unix))]
mod tests;

const READ_BUFFER: usize = 16 * 1024;
const EXIT_POLL: Duration = Duration::from_millis(10);
const ERROR_GRACE: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalExec;

impl HostExec for LocalExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        Box::pin(async move {
            let (ready, started) = oneshot::channel();
            spawn("nocterm-exec-start", move || {
                if !ready.is_canceled() {
                    // If the future was dropped during spawn, sending drops
                    // the output and the supervisor immediately cancels it.
                    let _ = ready.send(start(request));
                }
            })?;
            started
                .await
                .map_err(|error| ExecError::Failed(error.to_string()))?
        })
    }

    fn terminal(&self, request: TerminalRequest) -> Session {
        let launch = ShellLaunch {
            program: Some(request.program.program),
            args: request.program.args,
            integration: false,
            ..ShellLaunch::default()
        };
        crate::open(launch, false, request.term, request.size)
    }
}

fn start(request: ExecRequest) -> Result<ExecOutput, ExecError> {
    let mut process = process::Process::spawn(&request)?;
    let (Some(stdout), Some(stderr)) = (process.stdout(), process.stderr()) else {
        return Err(ExecError::Failed("no output pipe".into()));
    };
    if let (Some(input), Some(stdin)) = (request.stdin, process.stdin()) {
        write_input(stdin, input)?;
    }
    let (sink, output) = ExecOutput::channel();
    let sink = Arc::new(sink);
    let stopped = Arc::new(AtomicBool::new(false));
    let drained = read_output(stdout, sink.clone(), stopped.clone())?;
    let errors = read_errors(stderr, stopped.clone())?;
    spawn("nocterm-exec-wait", move || {
        // Observe the leader independently from pipe EOF: descendants may
        // retain both output pipes after the leader has exited.
        let natural = loop {
            if sink.is_closed() {
                break false;
            }
            match process.exited() {
                Ok(true) => break true,
                Ok(false) => thread::sleep(EXIT_POLL),
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
    })?;
    Ok(output)
}

fn write_input(mut stdin: ChildStdin, input: Vec<u8>) -> Result<(), ExecError> {
    spawn("nocterm-exec-in", move || {
        let _ = stdin.write_all(&input);
    })
}

fn read_output(
    mut stdout: ChildStdout,
    sink: Arc<ExecSink>,
    stopped: Arc<AtomicBool>,
) -> Result<oneshot::Receiver<()>, ExecError> {
    #[cfg(unix)]
    process::nonblocking(&stdout)?;
    let (finished, drained) = oneshot::channel();
    spawn("nocterm-exec-out", move || {
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
                    thread::sleep(EXIT_POLL);
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
) -> Result<std::sync::mpsc::Receiver<ErrorTail>, ExecError> {
    #[cfg(unix)]
    process::nonblocking(&stderr)?;
    let (send, errors) = std::sync::mpsc::channel();
    spawn("nocterm-exec-err", move || {
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
                    thread::sleep(EXIT_POLL);
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
