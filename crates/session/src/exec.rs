//! Programs run on the other end of a session, beside its terminal.
//!
//! A [`HostExec`] starts a non-interactive program on the session's host and
//! streams its standard output back. Nothing is echoed into the terminal and
//! no PTY is allocated, so readers such as the resource monitor can poll the
//! host without the user seeing it.
//!
//! A program that needs a terminal of its own, such as a log follower or an
//! interactive shell in a container, is started with [`HostExec::terminal`]
//! instead and talks through a [`Session`].

use std::{fmt, sync::Mutex};

use futures::{channel::oneshot, future::BoxFuture};

use crate::{Session, SessionError, TerminalRequest};

/// Chunks of output buffered before the program is slowed down.
const OUTPUT_BACKLOG: usize = 16;
/// The end of standard error kept to explain a failure.
const ERROR_TAIL: usize = 4 * 1024;

/// The result of starting a program, resolved off the calling thread.
pub type ExecFuture<T> = BoxFuture<'static, Result<T, ExecError>>;

/// Runs programs on a session's host.
pub trait HostExec: Send + Sync + 'static {
    /// Starts a program. Its standard output streams through the returned
    /// [`ExecOutput`]; the end of standard error comes with its [`ExecExit`].
    /// Dropping the output stops the program.
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput>;

    /// Starts a program on a terminal of its own, over the same connection.
    /// It talks through the returned session as a shell would; closing the
    /// session stops it.
    fn terminal(&self, request: TerminalRequest) -> Session {
        let _ = request;
        Session::failed(SessionError::Other(
            "running programs on a terminal is not supported here".into(),
        ))
    }
}

/// A program to run with separate arguments. Native local execution passes
/// arguments directly; SSH renders them as a POSIX command line, which requires
/// a POSIX-compatible remote command shell for arbitrary arguments.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ExecRequest {
    pub program: String,
    pub args: Vec<String>,
    /// Written to the program's standard input, which is then closed.
    pub stdin: Option<Vec<u8>>,
}

impl ExecRequest {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    /// The request as one POSIX command line, for transports that take a
    /// string. Plain words stay bare for compatibility with simple requests on
    /// non-POSIX shells. Arbitrary arguments are not guaranteed there: shells
    /// such as `cmd.exe` may interpret even unquoted characters like `%`.
    pub fn command_line(&self) -> String {
        let mut line = shell_word(&self.program);
        for arg in &self.args {
            line.push(' ');
            line.push_str(&shell_word(arg));
        }
        line
    }
}

impl fmt::Debug for ExecRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Arguments and input may be long scripts; they are not log material.
        formatter
            .debug_struct("ExecRequest")
            .field("program", &self.program)
            .field("args", &self.args.len())
            .field("stdin", &self.stdin.as_ref().map(Vec::len))
            .finish()
    }
}

fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./=:+,@%-".contains(&byte));
    if plain {
        word.to_owned()
    } else {
        crate::quote_posix(word)
    }
}

/// Why a program could not be started.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    #[error("running programs is not supported here")]
    Unsupported,
    #[error("the session is not connected")]
    Disconnected,
    /// The program is not installed, or not where it was looked for.
    #[error("{0}: no such program")]
    NotFound(String),
    #[error("could not start the program: {0}")]
    Failed(String),
}

/// How a program ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecExit {
    /// `None` when the host reported none: the program was stopped, killed
    /// by a signal, or the transport cannot tell.
    pub status: Option<u32>,
    /// The end of what the program wrote to standard error.
    pub stderr: String,
}

impl ExecExit {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// Why the program failed, in its own words when it gave any.
    pub fn error(&self) -> String {
        let said = self
            .stderr
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty());
        match (said, self.status) {
            (Some(line), _) => line.to_owned(),
            (None, Some(status)) => format!("the program exited with status {status}"),
            (None, None) => "the program was stopped".into(),
        }
    }
}

/// The end of a program's standard error, as transports collect it.
#[derive(Default)]
pub struct ErrorTail(Vec<u8>);

impl ErrorTail {
    /// Keeps `bytes`, dropping the oldest output beyond the bound.
    pub fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        let excess = self.0.len().saturating_sub(ERROR_TAIL);
        self.0.drain(..excess);
    }

    pub fn exit(self, status: Option<u32>) -> ExecExit {
        ExecExit {
            status,
            stderr: String::from_utf8_lossy(&self.0).into_owned(),
        }
    }
}

/// A short program's whole output, and how it ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Collected {
    pub stdout: Vec<u8>,
    pub exit: ExecExit,
}

impl Collected {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// The reading end of a running program's standard output.
pub struct ExecOutput {
    chunks: async_channel::Receiver<Vec<u8>>,
    exit: oneshot::Receiver<ExecExit>,
    // Never sent on: dropping it is what tells the sink to stop the program.
    _alive: async_channel::Sender<()>,
}

impl ExecOutput {
    /// The two ends of a program's output. Transports keep the sink.
    pub fn channel() -> (ExecSink, ExecOutput) {
        let (chunks, receiver) = async_channel::bounded(OUTPUT_BACKLOG);
        let (alive, watcher) = async_channel::bounded(1);
        let (exit, exited) = oneshot::channel();
        (
            ExecSink {
                chunks,
                alive: watcher,
                exit: Mutex::new(Some(exit)),
            },
            ExecOutput {
                chunks: receiver,
                exit: exited,
                _alive: alive,
            },
        )
    }

    /// The next chunk of output, or `None` once the program has finished.
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        self.chunks.recv().await.ok()
    }

    /// How the program ended. Output not read yet is discarded.
    pub async fn exit(mut self) -> ExecExit {
        while self.next().await.is_some() {}
        self.exit.await.unwrap_or_default()
    }

    /// The whole output and how the program ended. Output beyond `limit`
    /// bytes stops the program, whose end is then unknown.
    pub async fn collect(mut self, limit: usize) -> Collected {
        let mut stdout = Vec::new();
        while let Some(chunk) = self.next().await {
            stdout.extend(chunk);
            if stdout.len() > limit {
                stdout.truncate(limit);
                return Collected {
                    stdout,
                    exit: ExecExit::default(),
                };
            }
        }
        let exit = self.exit.await.unwrap_or_default();
        Collected { stdout, exit }
    }
}

impl fmt::Debug for ExecOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExecOutput")
    }
}

/// The writing end of a running program's standard output.
pub struct ExecSink {
    chunks: async_channel::Sender<Vec<u8>>,
    alive: async_channel::Receiver<()>,
    exit: Mutex<Option<oneshot::Sender<ExecExit>>>,
}

impl ExecSink {
    /// Delivers output. Returns `false` once the reader is gone, which is the
    /// transport's cue to stop the program.
    pub async fn send(&self, bytes: Vec<u8>) -> bool {
        self.chunks.send(bytes).await.is_ok()
    }

    /// Resolves once the reader has dropped its [`ExecOutput`].
    pub async fn closed(&self) {
        let _ = self.alive.recv().await;
    }

    pub fn is_closed(&self) -> bool {
        self.alive.is_closed()
    }

    /// Reports how the program ended. Only the first report counts; a sink
    /// dropped without one reports an unknown end.
    pub fn finish(&self, exit: ExecExit) {
        let sender = self
            .exit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(exit);
        }
    }
}

impl fmt::Debug for ExecSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExecSink")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_keeps_plain_words_bare() {
        let request = ExecRequest::new("powershell")
            .arg("-NoProfile")
            .arg("-EncodedCommand")
            .arg("SQBu+/=");
        assert_eq!(
            request.command_line(),
            "powershell -NoProfile -EncodedCommand SQBu+/="
        );
    }

    #[test]
    fn command_line_quotes_everything_else() {
        let request = ExecRequest::new("sh").arg("-c").arg("echo 'hi'").arg("");
        assert_eq!(request.command_line(), r#"sh -c 'echo '\''hi'\''' ''"#);
    }

    #[test]
    fn output_ends_with_the_sink_and_closes_it_when_dropped() {
        futures::executor::block_on(async {
            let (sink, mut output) = ExecOutput::channel();
            assert!(sink.send(b"a".to_vec()).await);
            assert_eq!(output.next().await.as_deref(), Some(&b"a"[..]));
            assert!(!sink.is_closed());
            drop(output);
            sink.closed().await;
            assert!(sink.is_closed());
            assert!(!sink.send(b"b".to_vec()).await);

            let (sink, mut output) = ExecOutput::channel();
            drop(sink);
            assert_eq!(output.next().await, None);
        });
    }

    #[test]
    fn collecting_reports_the_exit_and_bounds_the_output() {
        futures::executor::block_on(async {
            let (sink, output) = ExecOutput::channel();
            let program = async move {
                sink.send(b"one ".to_vec()).await;
                sink.send(b"two".to_vec()).await;
                let mut errors = ErrorTail::default();
                errors.push(b"warning\nno such container\n");
                sink.finish(errors.exit(Some(1)));
                sink.finish(ExecExit::default());
            };
            let (collected, ()) = futures::join!(output.collect(64), program);
            assert_eq!(collected.text(), "one two");
            assert_eq!(collected.exit.status, Some(1));
            assert!(!collected.exit.success());
            assert_eq!(collected.exit.error(), "no such container");

            let (sink, output) = ExecOutput::channel();
            let program = async move {
                sink.send(b"0123456789".to_vec()).await;
            };
            let (collected, ()) = futures::join!(output.collect(4), program);
            assert_eq!(collected.stdout, b"0123");
            assert_eq!(collected.exit, ExecExit::default());
        });
    }

    #[test]
    fn the_error_tail_keeps_the_newest_output() {
        let mut tail = ErrorTail::default();
        tail.push(&vec![b'a'; ERROR_TAIL]);
        tail.push(b"\nlast words");
        let exit = tail.exit(None);
        assert_eq!(exit.stderr.len(), ERROR_TAIL);
        assert_eq!(exit.error(), "last words");
        assert_eq!(
            ExecExit {
                status: Some(2),
                stderr: " \n".into()
            }
            .error(),
            "the program exited with status 2"
        );
    }
}
