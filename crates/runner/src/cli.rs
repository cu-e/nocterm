use crate::{RunnerError, bootstrap::State, config::Configuration, control};
use nocterm_core::{SharedCause, persist::ShutdownDeadline};
use std::{ffi::OsString, io::Read, path::PathBuf, time::Duration};
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
enum Command {
    Init,
    Enrol,
    Run,
    Health,
    Refresh,
}
struct Arguments {
    command: Command,
    configuration: PathBuf,
}
impl Arguments {
    fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Self, RunnerError> {
        let command = match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
            Some("init") => Command::Init,
            Some("enrol") => Command::Enrol,
            Some("run") => Command::Run,
            Some("health") => Command::Health,
            Some("refresh") => Command::Refresh,
            _ => return Err(RunnerError::Arguments),
        };
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
            return Err(RunnerError::Arguments);
        }
        let configuration = args
            .next()
            .map(PathBuf::from)
            .ok_or(RunnerError::Arguments)?;
        if args.next().is_some() {
            return Err(RunnerError::Arguments);
        }
        Ok(Self {
            command,
            configuration,
        })
    }
}
pub(crate) fn execute() -> Result<(), RunnerError> {
    let arguments = Arguments::parse(std::env::args_os().skip(1))?;
    let config = Configuration::read(&arguments.configuration)?;
    // Read the bounded secret before starting worker threads. The normal OS
    // signal disposition remains active during an interactive stdin wait.
    let ticket = if matches!(arguments.command, Command::Enrol) {
        Some(read_ticket(std::io::stdin().lock())?)
    } else {
        None
    };
    let shutdown = config.limits.shutdown();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()
        .map_err(RunnerError::Runtime)?;
    let completion = runtime.block_on(async move {
        if matches!(arguments.command, Command::Run) {
            return match State::open(config, false) {
                Ok(state) => control::run(state).await,
                Err(error) => Completion::finished(Err(error), shutdown),
            };
        }
        let result = command(arguments.command, config, ticket).await;
        Completion::finished(result, shutdown)
    });
    runtime.shutdown_timeout(completion.deadline.remaining());
    completion.result
}

async fn command(
    command: Command,
    config: Configuration,
    ticket: Option<Zeroizing<String>>,
) -> Result<(), RunnerError> {
    if matches!(command, Command::Health) {
        return control::health(&config.state).await;
    }
    let state = State::open(config, matches!(command, Command::Init))?;
    match command {
        Command::Init => state.print_identity(),
        Command::Enrol => {
            let ticket = ticket.ok_or(RunnerError::TicketFormat)?;
            tokio::time::timeout(
                state.limits().operation(),
                state.connection().redeem(ticket),
            )
            .await
            .map_err(|_| RunnerError::Timeout)??;
            println!("mode=control-plane\nexecution_ready=false\nenrolled=true");
            Ok(())
        }
        Command::Refresh => {
            tokio::time::timeout(state.limits().operation(), state.connection().restore())
                .await
                .map_err(|_| RunnerError::Timeout)??;
            tokio::time::timeout(state.limits().operation(), state.connection().refresh())
                .await
                .map_err(|_| RunnerError::Timeout)??;
            println!("mode=control-plane\nexecution_ready=false\ncredentials_durable=true");
            Ok(())
        }
        Command::Health | Command::Run => Err(RunnerError::Arguments),
    }
}
pub(crate) struct Completion {
    result: Result<(), RunnerError>,
    deadline: ShutdownDeadline,
}
impl Completion {
    pub(crate) fn finished(result: Result<(), RunnerError>, budget: Duration) -> Self {
        Self {
            result,
            deadline: ShutdownDeadline::new(budget),
        }
    }
    pub(crate) fn drained(result: Result<(), RunnerError>, deadline: ShutdownDeadline) -> Self {
        Self { result, deadline }
    }
}
fn read_ticket(mut input: impl Read) -> Result<Zeroizing<String>, RunnerError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(257));
    for _ in 0..258 {
        let mut next = [0u8; 1];
        if input
            .read(&mut next)
            .map_err(|error| RunnerError::Ticket(SharedCause::new(error)))?
            == 0
            || next[0] == b'\n'
        {
            break;
        }
        bytes.push(next[0]);
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    if bytes.is_empty() || bytes.len() > 256 || !bytes.iter().all(u8::is_ascii_graphic) {
        return Err(RunnerError::TicketFormat);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| RunnerError::TicketFormat)?;
    Ok(Zeroizing::new(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runner_cli_keeps_tickets_out_of_arguments_and_bounds_stdin_before_acceptance() {
        assert!(
            Arguments::parse(
                ["enrol", "--config", "private.toml"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_ok()
        );
        assert!(
            Arguments::parse(
                ["enrol", "--config", "private.toml", "secret"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
        assert_eq!(
            &**read_ticket(&b"actual-ticket\r\nignored"[..]).unwrap(),
            "actual-ticket"
        );
        assert!(read_ticket(&vec![b'a'; 257][..]).is_err());
        assert!(read_ticket(&b"contains space\n"[..]).is_err());
        assert!(read_ticket(&b"\n"[..]).is_err());
    }
}
