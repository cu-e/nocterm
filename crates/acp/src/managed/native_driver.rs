//! The original process, protocol and stderr lifetime run in one paid native task.
use super::{
    budget::SourceBudget, channel::Sender, diagnostics::Diagnostics, driver::DriverTask,
    handler::Incoming, native_session::Session, stderr::Tail,
};
use agent_client_protocol::managed::Shared;
use futures::{
    AsyncReadExt as _,
    future::{Either, select},
};
use nocterm_ai::{connection::ManagedAgentEvent, history::source::SourceErrorSlot};
use std::{sync::Arc, time::Duration};
pub(super) struct NativeDriver {
    process: ProcessOwner,
    driver: DriverTask<async_process::ChildStdout, async_process::ChildStdin, Incoming>,

    tail: Shared<std::sync::Mutex<Tail>, SourceBudget>,
    session: Session,

    events: Sender<ManagedAgentEvent>,
    diagnostics: Shared<Diagnostics, SourceBudget>,
    ticket: SourceErrorSlot<super::native_connect::Cause>,
    budget: SourceBudget,
}
impl NativeDriver {
    pub(super) fn new(
        process: ProcessOwner,
        driver: DriverTask<async_process::ChildStdout, async_process::ChildStdin, Incoming>,
        session: Session,
        events: Sender<ManagedAgentEvent>,
        diagnostics: Shared<Diagnostics, SourceBudget>,
        ticket: SourceErrorSlot<super::native_connect::Cause>,
        budget: SourceBudget,
    ) -> Result<
        Self,
        agent_client_protocol::managed::ManagedError<
            nocterm_ai::model::json::deferred::JsonDecodeError,
        >,
    > {
        let tail = Shared::new(std::sync::Mutex::new(Tail::new(budget.clone())), &budget)?;
        Ok(Self {
            process,
            driver,
            tail,
            session,
            events,
            diagnostics,
            ticket,
            budget,
        })
    }
    pub(super) async fn run(self) {
        let Self {
            process,
            driver,
            tail,
            session,
            events,
            diagnostics,
            ticket,
            budget,
        } = self;
        let ProcessOwner {
            mut child,
            mut stderr,
            group,
        } = process;
        let capture = async {
            let mut buffer = [0u8; 4096];
            while let Ok(count) = stderr.read(&mut buffer).await {
                if count == 0 {
                    break;
                }
                tail.get()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(&buffer[..count])
                    .inspect_err(|_error| {
                        group.kill();
                    })?;
            }
            Ok::<
                _,
                agent_client_protocol::managed::ManagedError<
                    nocterm_ai::model::json::deferred::JsonDecodeError,
                >,
            >(())
        };
        let process = async {
            let protocol = async {
                futures::pin_mut!(driver);
                let session = session.run();
                futures::pin_mut!(session);
                match select(driver, session).await {
                    Either::Left((result, _)) => result,
                    Either::Right(_) => Ok(()),
                }
            };
            futures::pin_mut!(protocol);
            let status = child.status();
            futures::pin_mut!(status);
            match select(protocol, status).await {
                Either::Left((result, status)) => {
                    let _ = group.shutdown_native().await;
                    (result, status.await.ok())
                }
                Either::Right((status, protocol)) => {
                    let _ = async_io::Timer::after(Duration::from_millis(250)).await;
                    let _ = group.shutdown_native().await;
                    (protocol.await, status.ok())
                }
            }
        };
        let (capture, (result, status)) = futures::future::join(capture, process).await;
        let _ = async_io::Timer::after(Duration::from_millis(50)).await;
        let event = final_event(
            capture,
            result,
            status,
            &tail,
            &diagnostics,
            &budget,
            ticket,
        );
        let _ = events.publish_final(event);
        events.close();
    }
}

/// The terminal event: source failures fail; otherwise the redacted exit tail.
fn final_event(
    capture: Result<
        (),
        agent_client_protocol::managed::ManagedError<
            nocterm_ai::model::json::deferred::JsonDecodeError,
        >,
    >,
    result: Result<(), super::driver::DriverError>,
    status: Option<std::process::ExitStatus>,
    tail: &Shared<std::sync::Mutex<Tail>, SourceBudget>,
    diagnostics: &Shared<Diagnostics, SourceBudget>,
    budget: &SourceBudget,
    ticket: SourceErrorSlot<super::native_connect::Cause>,
) -> ManagedAgentEvent {
    match capture {
        Err(error) => {
            ManagedAgentEvent::Failed(super::native_connect::failure(error.into(), ticket))
        }
        Ok(()) => match result {
            Err(error) if error.source_failure() => {
                ManagedAgentEvent::Failed(super::native_connect::failure(error.into(), ticket))
            }
            result => match exit_tail(tail, diagnostics, budget, result) {
                Ok(stderr_tail) => ManagedAgentEvent::Exited {
                    code: status.and_then(|value| value.code()),
                    stderr_tail,
                },
                Err(error) => {
                    ManagedAgentEvent::Failed(super::native_connect::failure(error, ticket))
                }
            },
        },
    }
}

/// The redacted stderr tail, or the protocol failure text when stderr was empty.
#[expect(
    clippy::result_large_err,
    reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
)]
fn exit_tail(
    tail: &Shared<std::sync::Mutex<Tail>, SourceBudget>,
    diagnostics: &Shared<Diagnostics, SourceBudget>,
    budget: &SourceBudget,
    result: Result<(), super::driver::DriverError>,
) -> Result<nocterm_ai::model::native::NativeOwned<String>, super::native_connect::Cause> {
    let source = tail
        .get()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .decoded()?;
    let source = if source.text().is_empty() {
        match result {
            Err(error) => {
                // The empty tail is freed before the replacement text is paid.
                drop(source);
                super::diagnostic_text::format(format_args!("{error}"), budget)?
            }
            Ok(()) => source,
        }
    } else {
        source
    };
    Ok(diagnostics.get().owned(source)?)
}

pub(super) struct ProcessOwner {
    child: async_process::Child,
    stderr: async_process::ChildStderr,
    group: Arc<crate::process::ProcessGroup>,
}
impl ProcessOwner {
    pub(super) fn new(
        child: async_process::Child,
        stderr: async_process::ChildStderr,
        group: Arc<crate::process::ProcessGroup>,
    ) -> Self {
        Self {
            child,
            stderr,
            group,
        }
    }
}
