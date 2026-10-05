//! Programs run on the other end of a session, beside its terminal.
//!
//! A [`HostExec`] starts a non-interactive program on the session's host and
//! streams its standard output back. Nothing is echoed into the terminal and
//! no PTY is allocated, so readers such as the resource monitor can poll the
//! host without the user seeing it.

use std::fmt;

use futures::future::BoxFuture;

/// Chunks of output buffered before the program is slowed down.
const OUTPUT_BACKLOG: usize = 16;

/// The result of starting a program, resolved off the calling thread.
pub type ExecFuture<T> = BoxFuture<'static, Result<T, ExecError>>;

/// Runs programs on a session's host.
pub trait HostExec: Send + Sync + 'static {
    /// Starts a program. Its standard output streams through the returned
    /// [`ExecOutput`]; standard error is discarded. Dropping the output stops
    /// the program.
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput>;
}

/// A program to run, with its arguments passed verbatim.
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

    pub fn stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    /// The request as one POSIX command line, for transports that take a
    /// string. Plain words stay bare so non-POSIX shells (`cmd.exe`) read
    /// them the same way.
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
    #[error("could not start the program: {0}")]
    Failed(String),
}

/// The reading end of a running program's standard output.
pub struct ExecOutput {
    chunks: async_channel::Receiver<Vec<u8>>,
    // Never sent on: dropping it is what tells the sink to stop the program.
    _alive: async_channel::Sender<()>,
}

impl ExecOutput {
    /// The two ends of a program's output. Transports keep the sink.
    pub fn channel() -> (ExecSink, ExecOutput) {
        let (chunks, receiver) = async_channel::bounded(OUTPUT_BACKLOG);
        let (alive, watcher) = async_channel::bounded(1);
        (
            ExecSink {
                chunks,
                alive: watcher,
            },
            ExecOutput {
                chunks: receiver,
                _alive: alive,
            },
        )
    }

    /// The next chunk of output, or `None` once the program has finished.
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        self.chunks.recv().await.ok()
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
}
