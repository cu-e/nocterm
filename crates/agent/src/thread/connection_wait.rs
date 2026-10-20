//! Typed reconnect/open orchestration failures become tool results only at delivery.
use nocterm_workspace_ports::TerminalError;
#[derive(Debug, thiserror::Error)]
pub(super) enum ConnectionWaitError {
    #[error("The server was detached before it connected.")]
    Detached,
    #[error("{0}")]
    Terminal(#[from] TerminalError),
    #[error("The workspace is unavailable.")]
    WorkspaceClosed,
    #[error("Could not open a session for this server.")]
    OpenFailed,
    #[error("The chat stopped before the server connected.")]
    Stopped,
    #[error("The session was closed before it connected.")]
    SessionClosed,
    #[error("The chat was closed.")]
    ChatClosed,
    #[error("Could not connect to the server.")]
    ConnectFailed,
    #[error("Connecting to the server timed out.")]
    Timeout,
    #[error(
        "The server is waiting for the user to sign in; ask them to finish signing in, then call list_terminals."
    )]
    VisiblePromptTimeout,
    #[error(
        "The server is waiting for the user to sign in from the chat or unlock the vault; ask them to, then call open_terminal again."
    )]
    HiddenPromptTimeout,
}
impl ConnectionWaitError {
    pub(super) fn into_failure(self) -> nocterm_ai::ToolFailure {
        match self {
            Self::WorkspaceClosed
            | Self::OpenFailed
            | Self::ConnectFailed
            | Self::Timeout
            | Self::Terminal(_) => super::tools::unreachable(&self.to_string()),
            _ => self.to_string().into(),
        }
    }
}

use super::AgentThread;
use gpui_kit::{AppContext as _, AsyncApp, WeakEntity};
use nocterm_workspace_ports::{ExpectedSession, TerminalStatus};
use std::time::{Duration, Instant};
/// Authority held by one waiter, never inferred from a replacement session.
pub(in crate::thread) struct WaitTarget {
    seat: crate::seat::Seat,
    window: gpui_kit::AnyWindowHandle,
    expected: ExpectedSession,
    ticket: nocterm_ai::session::Ticket,
    registration: Option<u64>,
}
impl WaitTarget {
    pub(in crate::thread) fn probe(
        &self,
        entry: Option<(
            String,
            nocterm_workspace_ports::TerminalEntry,
            nocterm_ai::context::TerminalDescriptor,
        )>,
        cx: &gpui_kit::App,
    ) -> Result<
        (
            Option<TerminalStatus>,
            Option<super::SignInWait>,
            nocterm_ai::context::TerminalDescriptor,
        ),
        ConnectionWaitError,
    > {
        let Some((_, entry, descriptor)) = entry else {
            return Err(ConnectionWaitError::SessionClosed);
        };
        let item = self.expected.item();
        let info = entry.capabilities.state().info(cx);
        if info
            .as_ref()
            .is_none_or(|info| info.expected != self.expected)
        {
            return Err(ConnectionWaitError::Terminal(TerminalError::StaleSession));
        }
        let status = info.as_ref().map(|info| info.status);
        // A password is asked in the chat; the session stays hidden.
        let wait = info.and_then(|info| {
            let vault = info.status == TerminalStatus::AwaitingVault;
            let user = info.status == TerminalStatus::AwaitingUser;
            let prompts = entry.capabilities.prompts()?.clone();
            let prompt = prompts
                .pending(cx)
                .filter(|prompt| prompt.token.expected() == self.expected && (vault || user))?;
            Some(super::SignInWait::new(
                item,
                entry.title.clone(),
                prompts,
                prompt,
                vault,
            ))
        });
        Ok((status, wait, descriptor))
    }

    pub(in crate::thread) fn expected(&self) -> ExpectedSession {
        self.expected
    }
    pub(in crate::thread) fn ticket(&self) -> nocterm_ai::session::Ticket {
        self.ticket
    }
    pub(in crate::thread) fn registration(&self) -> Option<u64> {
        self.registration
    }
    pub(in crate::thread) fn seat(&self) -> &crate::seat::Seat {
        &self.seat
    }
    pub(in crate::thread) fn window(&self) -> gpui_kit::AnyWindowHandle {
        self.window
    }

    pub(in crate::thread) fn new(
        seat: crate::seat::Seat,
        window: gpui_kit::AnyWindowHandle,
        expected: ExpectedSession,
        ticket: nocterm_ai::session::Ticket,
        registration: Option<u64>,
    ) -> Self {
        Self {
            seat,
            window,
            expected,
            ticket,
            registration,
        }
    }
}

/// One finite waiter owns timing and promotion decisions for its original receipt.
pub(in crate::thread) struct ConnectionWaitOperation {
    target: WaitTarget,
    started: Instant,
    shown: bool,
    waiting: bool,
}
impl ConnectionWaitOperation {
    pub(in crate::thread) fn new(target: WaitTarget) -> Self {
        Self {
            target,
            started: Instant::now(),
            shown: false,
            waiting: false,
        }
    }
    pub(in crate::thread) async fn run(
        mut self,
        thread: &WeakEntity<AgentThread>,
        cx: &mut AsyncApp,
    ) -> Result<serde_json::Value, ConnectionWaitError> {
        let result = loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let step = thread.update(cx, |thread, cx| {
                thread.poll_connection_wait(&self.target, cx)
            });
            let step = match step {
                Ok(Ok(step)) => step,
                Ok(Err(error)) => break Err(error),
                Err(_) => break Err(ConnectionWaitError::ChatClosed),
            };
            if let Some(result) = self.advance(step, cx) {
                break result;
            }
        };
        let _ = thread.update(cx, |thread, cx| {
            thread.on_connection_wait_complete(self.target.expected, cx)
        });
        result
    }
    fn advance(
        &mut self,
        step: (
            Option<TerminalStatus>,
            bool,
            nocterm_ai::context::TerminalDescriptor,
        ),
        cx: &mut AsyncApp,
    ) -> Option<Result<serde_json::Value, ConnectionWaitError>> {
        match step {
            (Some(TerminalStatus::Connected), _, descriptor) => {
                return Some(Ok(serde_json::json!({ "terminal": descriptor })));
            }
            (_, true, _) => self.waiting = true,
            (Some(TerminalStatus::AwaitingUser), false, _) if !self.shown => {
                self.shown = true;
                let seat = self.target.seat.clone();
                let item = self.target.expected.item();
                let _ = cx.update_window(self.target.window, |_, window, cx| {
                    seat.show_background(item, window, cx)
                });
            }
            (Some(TerminalStatus::Closed) | None, _, _) => {
                return Some(Err(ConnectionWaitError::ConnectFailed));
            }
            _ => {}
        }
        let limit = if self.shown || self.waiting {
            Duration::from_secs(180)
        } else {
            Duration::from_secs(60)
        };
        (self.started.elapsed() >= limit).then(|| {
            Err(if self.shown {
                ConnectionWaitError::VisiblePromptTimeout
            } else if self.waiting {
                ConnectionWaitError::HiddenPromptTimeout
            } else {
                ConnectionWaitError::Timeout
            })
        })
    }
}
