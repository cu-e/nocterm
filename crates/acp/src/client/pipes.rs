//! The three standard streams of a freshly spawned agent.
use async_process::{Child, ChildStderr, ChildStdin, ChildStdout};

use super::error::StartupError;

pub(super) struct Pipes {
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: ChildStderr,
}
impl Pipes {
    pub(super) fn take(child: &mut Child) -> Result<Self, StartupError> {
        let stdin = child
            .stdin
            .take()
            .ok_or(StartupError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(StartupError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(StartupError::MissingPipe("stderr"))?;
        Ok(Self {
            stdin,
            stdout,
            stderr,
        })
    }
    pub(super) fn split(self) -> (ChildStdin, ChildStdout, ChildStderr) {
        (self.stdin, self.stdout, self.stderr)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use async_process::{Command, Stdio};

    fn child(stdin: Stdio, stdout: Stdio, stderr: Stdio) -> Child {
        Command::new("/bin/true")
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap()
    }

    #[test]
    fn each_missing_pipe_is_named_exactly() {
        for (name, mut child) in [
            (
                "stdin",
                child(Stdio::null(), Stdio::piped(), Stdio::piped()),
            ),
            (
                "stdout",
                child(Stdio::piped(), Stdio::null(), Stdio::piped()),
            ),
            (
                "stderr",
                child(Stdio::piped(), Stdio::piped(), Stdio::null()),
            ),
        ] {
            let error = Pipes::take(&mut child).err().unwrap();
            assert_eq!(error.to_string(), format!("Agent {name} unavailable"));
        }
    }

    #[test]
    fn complete_pipes_are_taken_from_the_child() {
        let mut child = child(Stdio::piped(), Stdio::piped(), Stdio::piped());
        assert!(Pipes::take(&mut child).is_ok());
        assert!(child.stdin.is_none() && child.stdout.is_none() && child.stderr.is_none());
    }
}
