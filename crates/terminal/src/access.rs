use gpui_kit::{App, WeakEntity};
use nocterm_session::{HostExec, Prompt, Secret};
use nocterm_workspace::{
    SignInPrompt, TerminalAccess, TerminalCapability, TerminalError, TerminalInfo, TerminalStatus,
    TerminalText, TextRequest,
};
use std::sync::Arc;

use crate::{Status, Terminal, credentials::describe};

pub(crate) struct Access(WeakEntity<Terminal>);

impl Access {
    pub(crate) fn new(terminal: WeakEntity<Terminal>) -> Self {
        Self(terminal)
    }
}

impl TerminalAccess for Access {
    fn info(&self, cx: &App) -> Option<TerminalInfo> {
        let entity = self.0.upgrade()?;
        let terminal = entity.read(cx);
        Some(TerminalInfo {
            title: terminal.spec().title.clone(),
            local: terminal.is_local(),
            target: (!terminal.is_local()).then(|| terminal.spec().target.clone()),
            profile: terminal.spec().profile.clone(),
            status: if terminal.awaiting_vault() {
                TerminalStatus::AwaitingVault
            } else if terminal.prompt().is_some() {
                TerminalStatus::AwaitingUser
            } else {
                match terminal.status() {
                    Status::Connecting(_) => TerminalStatus::Connecting,
                    Status::Connected => TerminalStatus::Connected,
                    Status::Closed(_) => TerminalStatus::Closed,
                }
            },
            cwd: terminal.cwd(),
            at_prompt: terminal.agent_prompt_state().0,
            dirty_input: terminal.agent_prompt_state().1,
            alt_screen: terminal.emulator().modes().alt_screen,
            generation: terminal.emulator().generation(),
            sign_in: match terminal.prompt() {
                Some(Prompt::Secret { request, .. }) => {
                    let (label, retry, masked) = describe(request);
                    Some(SignInPrompt {
                        label: label.into(),
                        retry,
                        masked,
                    })
                }
                _ => None,
            },
        })
    }

    fn read(&self, request: TextRequest, cx: &App) -> Result<TerminalText, TerminalError> {
        let entity = self.0.upgrade().ok_or(TerminalError::Gone)?;
        let terminal = entity.read(cx);
        if !terminal.is_connected() || terminal.prompt().is_some() {
            return Err(TerminalError::Unavailable);
        }
        let tail = terminal.emulator().text(nocterm_vt::TextQuery {
            max_lines: request.max_lines.min(2000),
            max_bytes: request.max_bytes.min(64 * 1024),
            since_line: request.since_line,
        });
        Ok(TerminalText {
            text: tail.text,
            first_line: tail.first_line,
            next_line: tail.resume_line,
            truncated: tail.truncated,
            alt_screen: tail.alt_screen,
        })
    }

    fn send_text(&self, text: &str, cx: &mut App) -> Result<(), TerminalError> {
        self.0
            .update(cx, |terminal, cx| terminal.agent_send(text, false, cx))
            .map_err(|_| TerminalError::Gone)?
    }

    fn run_command(&self, command: &str, cx: &mut App) -> Result<(), TerminalError> {
        self.0
            .update(cx, |terminal, cx| terminal.agent_send(command, true, cx))
            .map_err(|_| TerminalError::Gone)?
    }

    fn executor(&self, cx: &App) -> Result<Arc<dyn HostExec>, TerminalError> {
        let entity = self.0.upgrade().ok_or(TerminalError::Gone)?;
        let terminal = entity.read(cx);
        if !terminal.is_connected() || terminal.prompt().is_some() || terminal.awaiting_vault() {
            return Err(TerminalError::Unavailable);
        }
        terminal.agent_executor().ok_or(TerminalError::Unsupported(
            TerminalCapability::SessionExecution,
        ))
    }

    fn begin_live_command(
        &self,
        command: &str,
        cx: &mut App,
    ) -> Result<nocterm_workspace::LiveCommandLease, TerminalError> {
        self.0
            .update(cx, |terminal, cx| terminal.begin_live_command(command, cx))
            .map_err(|_| TerminalError::Gone)?
    }

    fn paste_snippet(&self, text: &str, execute: bool, cx: &mut App) -> Result<(), TerminalError> {
        self.0
            .update(cx, |terminal, cx| terminal.paste_snippet(text, execute, cx))
            .map_err(|_| TerminalError::Gone)?
    }

    fn answer_sign_in(&self, answer: String, cx: &mut App) -> Result<(), TerminalError> {
        self.0
            .update(cx, |terminal, cx| {
                if !matches!(terminal.prompt(), Some(Prompt::Secret { .. })) {
                    return Err(TerminalError::SignInNoLongerRequested);
                }
                terminal.answer_secret_and_remember(Secret::new(answer), false, cx);
                Ok(())
            })
            .map_err(|_| TerminalError::Gone)?
    }

    fn reconnect(&self, cx: &mut App) -> Result<(), TerminalError> {
        self.0
            .update(cx, |terminal, cx| {
                if terminal.is_local() || terminal.is_command() {
                    return Err(TerminalError::OnlyRemoteReconnect);
                }
                if matches!(terminal.status(), Status::Closed(_)) {
                    terminal.reconnect(cx);
                }
                Ok(())
            })
            .map_err(|_| TerminalError::Gone)?
    }
}
