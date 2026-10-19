//! The protocol driver thread: handshake, command surface and exit reporting.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::{Agent, ConnectionTo, Lines};
use agent_client_protocol_schema::v1 as acp;
use async_process::{ChildStdin, ChildStdout};
use futures::{
    channel::oneshot,
    future::{Either, select},
};
use nocterm_ai::AgentConnection;

use super::{
    commands::Commands,
    error::StartupError,
    exit_report::{EventQueue, ExitReport},
    handlers::client_builder_with_models,
    handshake::{Handshake, Introduction},
    stderr_tail::StderrTail,
};
use crate::{
    lines,
    models::Models,
    process::{ProcessGroup, Spawned},
};

pub(super) type Ready = oneshot::Sender<Result<AgentConnection, StartupError>>;

/// Everything the driver thread owns for one agent subprocess.
pub(super) struct Driver {
    process: Spawned,
    stdin: ChildStdin,
    stdout: ChildStdout,
    tail: StderrTail,
    events: EventQueue,
    handshake: Handshake,
    ready: Ready,
}
impl Driver {
    pub(super) fn new(
        process: Spawned,
        (stdin, stdout): (ChildStdin, ChildStdout),
        tail: StderrTail,
        events: EventQueue,
        handshake: Handshake,
        ready: Ready,
    ) -> Self {
        Self {
            process,
            stdin,
            stdout,
            tail,
            events,
            handshake,
            ready,
        }
    }
    pub(super) fn spawn(self) -> Result<(), StartupError> {
        std::thread::Builder::new()
            .name("nocterm-acp".into())
            .spawn(move || futures::executor::block_on(self.run()))
            .map(drop)
            .map_err(StartupError::ThreadSpawn)
    }
    async fn run(self) {
        let Self {
            mut process,
            stdin,
            stdout,
            tail,
            events,
            handshake,
            ready,
        } = self;
        let models = Arc::new(Mutex::new(Models::default()));
        let (stop, stopped) = async_channel::bounded(1);
        let session = Session {
            handshake,
            ready,
            stop,
            stopped,
            group: process.group.clone(),
            secrets: process.secrets.clone(),
            models: models.clone(),
            events: events.clone(),
        };
        let protocol = client_builder_with_models(events.sender(), models).connect_with(
            Lines::new(
                Box::pin(lines::outgoing(stdin)),
                Box::pin(lines::incoming(stdout)),
            ),
            async move |connection: ConnectionTo<Agent>| session.serve(connection).await,
        );
        let (result, status) =
            match select(Box::pin(protocol), Box::pin(process.child.status())).await {
                Either::Left((result, status)) => {
                    let _ = process.group.shutdown().await;
                    (result, status.await.ok())
                }
                Either::Right((status, protocol)) => {
                    // Descendants can retain stdout after the leader exits.
                    // Drain final protocol events, then terminate the entire group.
                    let _ = async_io::Timer::after(Duration::from_millis(250)).await;
                    let _ = process.group.shutdown().await;
                    (protocol.await, status.ok())
                }
            };
        let _ = async_io::Timer::after(Duration::from_millis(50)).await;
        events.finish(ExitReport::new(
            status.and_then(|status| status.code()),
            &tail,
            &process.secrets,
            result.err().map(|error| error.to_string()),
        ));
    }
}

/// The connected half of the driver: handshake, then serve until stopped or closed.
struct Session {
    handshake: Handshake,
    ready: Ready,
    stop: async_channel::Sender<()>,
    stopped: async_channel::Receiver<()>,
    group: Arc<ProcessGroup>,
    secrets: Arc<Vec<String>>,
    models: Arc<Mutex<Models>>,
    events: EventQueue,
}
impl Session {
    async fn serve(self, connection: ConnectionTo<Agent>) -> Result<(), acp::Error> {
        let Introduction { info, support } = match self.handshake.run(&connection).await {
            Ok(introduction) => introduction,
            Err(error) => {
                let _ = self.ready.send(Err(error));
                return Ok(());
            }
        };
        let commands = Arc::new(Commands::new(
            connection.clone(),
            self.stop,
            self.group,
            self.secrets,
            support,
            self.models,
            self.events.sender(),
        ));
        let connected = AgentConnection {
            info,
            commands,
            events: self.events.receiver(),
        };
        if self.ready.send(Ok(connected)).is_err() {
            return Ok(());
        }
        let _ = select(
            Box::pin(self.stopped.recv()),
            Box::pin(connection.incoming_closed()),
        )
        .await;
        Ok(())
    }
}
