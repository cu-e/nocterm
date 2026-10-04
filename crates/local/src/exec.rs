//! Programs run on this computer, for readers that ask a host about itself.

use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
};

use futures::{FutureExt, channel::oneshot, executor::block_on, select};
use nocterm_session::{ExecError, ExecFuture, ExecOutput, ExecRequest, HostExec};

const READ_BUFFER: usize = 16 * 1024;

/// Runs programs on the local computer, without a console window.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalExec;

impl HostExec for LocalExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let started = start(request);
        Box::pin(async move { started })
    }
}

fn start(request: ExecRequest) -> Result<ExecOutput, ExecError> {
    let mut command = Command::new(&request.program);
    command
        .args(&request.args)
        .stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Its own group, so stopping it also stops whatever it started.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|error| ExecError::Failed(format!("{}: {error}", request.program)))?;
    let Some(mut stdout) = child.stdout.take() else {
        stop(&mut child);
        return Err(ExecError::Failed("no output pipe".into()));
    };
    if let (Some(input), Some(mut stdin)) = (request.stdin, child.stdin.take()) {
        // A separate thread: a large input must not wait for output to drain.
        // Dropping the pipe afterwards closes the program's input.
        spawn("nocterm-exec-in", move || {
            let _ = stdin.write_all(&input);
        })?;
    }

    let (sink, output) = ExecOutput::channel();
    let sink = Arc::new(sink);
    let (finished, ended) = oneshot::channel::<()>();
    let reader = sink.clone();
    spawn("nocterm-exec-out", move || {
        let mut buffer = vec![0; READ_BUFFER];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if !block_on(reader.send(buffer[..read].to_vec())) {
                        break;
                    }
                }
            }
        }
        drop(finished);
    })?;
    spawn("nocterm-exec-wait", move || {
        block_on(async {
            select! {
                () = sink.closed().fuse() => {},
                _ = ended.fuse() => {},
            }
        });
        stop(&mut child);
    })?;
    Ok(output)
}

fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> Result<(), ExecError> {
    thread::Builder::new()
        .name(name.into())
        .spawn(body)
        .map(drop)
        .map_err(|error| ExecError::Failed(error.to_string()))
}

/// Ends the program, if it still runs, and reaps it.
fn stop(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        #[cfg(unix)]
        {
            use nix::{sys::signal, unistd::Pid};
            if let Ok(group) = i32::try_from(child.id()) {
                let _ = signal::killpg(Pid::from_raw(group), signal::Signal::SIGKILL);
            }
        }
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn collect(mut output: ExecOutput) -> String {
        block_on(async {
            let mut collected = Vec::new();
            while let Some(chunk) = output.next().await {
                collected.extend(chunk);
            }
            String::from_utf8(collected).unwrap()
        })
    }

    #[test]
    fn runs_a_script_from_its_input() {
        let request = ExecRequest::new("sh")
            .arg("-s")
            .stdin("echo one; echo \"$((40 + 2))\"\n");
        let output = block_on(LocalExec.exec(request)).unwrap();
        assert_eq!(collect(output), "one\n42\n");
    }

    #[test]
    fn dropping_the_output_stops_the_program_and_its_children() {
        let marker = tempfile::tempdir().unwrap();
        let file = marker.path().join("alive");
        let script = format!(
            "echo started; (sleep 0.3; touch '{}') & wait",
            file.display()
        );
        let request = ExecRequest::new("sh").arg("-c").arg(script);
        let mut output = block_on(LocalExec.exec(request)).unwrap();
        assert_eq!(block_on(output.next()).as_deref(), Some(&b"started\n"[..]));
        drop(output);
        thread::sleep(Duration::from_millis(600));
        assert!(!file.exists(), "the background child was stopped too");
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let started = Instant::now();
        let error = block_on(LocalExec.exec(ExecRequest::new("/nonexistent/nocterm"))).unwrap_err();
        assert!(matches!(error, ExecError::Failed(_)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
