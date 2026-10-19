use gpui_kit::{App, WeakEntity};
use nocterm_session::{HostExec, Prompt, Secret};
use nocterm_workspace::{
    ExpectedSession, LiveCommands, RemoteReconnect, SignInPrompt, SignInPrompts,
    TerminalCapability, TerminalError, TerminalExecution, TerminalInfo, TerminalInput,
    TerminalState, TerminalStatus, TerminalText, TerminalTextSource, TextRequest, UserPromptToken,
    UserTerminalPaste,
};
use std::sync::Arc;

use crate::{Status, Terminal, TerminalView, credentials::describe};

pub(crate) struct Access {
    owner: WeakEntity<TerminalView>,
    terminal: WeakEntity<Terminal>,
}

impl Access {
    pub(crate) fn new(owner: WeakEntity<TerminalView>, terminal: WeakEntity<Terminal>) -> Self {
        Self { owner, terminal }
    }
    fn item(&self) -> Result<gpui_kit::EntityId, TerminalError> {
        self.owner
            .upgrade()
            .map(|owner| owner.entity_id())
            .ok_or(TerminalError::Gone)
    }
    fn check(&self, expected: ExpectedSession, terminal: &Terminal) -> Result<(), TerminalError> {
        if self.item()? != expected.item() || terminal.connection_epoch() != expected.epoch() {
            return Err(TerminalError::StaleSession);
        }
        Ok(())
    }
}

impl nocterm_workspace::ConnectionFactSession for Access {
    fn current(&self, cx: &App) -> Option<nocterm_workspace::FactSessionSnapshot> {
        let item = self.item().ok()?;
        let entity = self.terminal.upgrade()?;
        let terminal = entity.read(cx);
        if terminal.is_local() {
            return None;
        }
        Some(nocterm_workspace::FactSessionSnapshot {
            expected: ExpectedSession::new(item, terminal.connection_epoch()),
            session: terminal.session_context(),
            profile: terminal.spec().profile.clone(),
        })
    }
}

impl TerminalState for Access {
    fn info(&self, cx: &App) -> Option<TerminalInfo> {
        let item = self.item().ok()?;
        let entity = self.terminal.upgrade()?;
        let terminal = entity.read(cx);
        let expected = ExpectedSession::new(item, terminal.connection_epoch());
        Some(TerminalInfo {
            expected,
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
                        token: UserPromptToken::new(expected, terminal.prompt_epoch()),
                        label: label.into(),
                        retry,
                        masked,
                    })
                }
                _ => None,
            },
        })
    }
}

impl TerminalTextSource for Access {
    fn read(
        &self,
        expected: ExpectedSession,
        request: TextRequest,
        cx: &App,
    ) -> Result<TerminalText, TerminalError> {
        self.item()?;
        let entity = self.terminal.upgrade().ok_or(TerminalError::Gone)?;
        let terminal = entity.read(cx);
        self.check(expected, terminal)?;
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
}

impl TerminalInput for Access {
    fn send_text(
        &self,
        expected: ExpectedSession,
        text: &str,
        cx: &mut App,
    ) -> Result<(), TerminalError> {
        self.item()?;
        self.terminal
            .update(cx, |terminal, cx| {
                self.check(expected, terminal)?;
                terminal.agent_send(text, false, cx)
            })
            .map_err(|_| TerminalError::Gone)?
    }
}

impl TerminalExecution for Access {
    fn executor(
        &self,
        expected: ExpectedSession,
        cx: &App,
    ) -> Result<Arc<dyn HostExec>, TerminalError> {
        self.item()?;
        let entity = self.terminal.upgrade().ok_or(TerminalError::Gone)?;
        let terminal = entity.read(cx);
        self.check(expected, terminal)?;
        if !terminal.is_connected() || terminal.prompt().is_some() || terminal.awaiting_vault() {
            return Err(TerminalError::Unavailable);
        }
        terminal.agent_executor().ok_or(TerminalError::Unsupported(
            TerminalCapability::SessionExecution,
        ))
    }
}

impl LiveCommands for Access {
    fn begin(
        &self,
        expected: ExpectedSession,
        command: &str,
        cx: &mut App,
    ) -> Result<nocterm_workspace::LiveCommandLease, TerminalError> {
        self.item()?;
        self.terminal
            .update(cx, |terminal, cx| {
                self.check(expected, terminal)?;
                terminal.begin_live_command(command, cx)
            })
            .map_err(|_| TerminalError::Gone)?
    }
}

impl UserTerminalPaste for Access {
    fn paste(
        &self,
        expected: ExpectedSession,
        text: &str,
        execute: bool,
        cx: &mut App,
    ) -> Result<(), TerminalError> {
        self.item()?;
        self.terminal
            .update(cx, |terminal, cx| {
                self.check(expected, terminal)?;
                terminal.paste_snippet(text, execute, cx)
            })
            .map_err(|_| TerminalError::Gone)?
    }
}

impl SignInPrompts for Access {
    fn pending(&self, cx: &App) -> Option<SignInPrompt> {
        self.info(cx)?.sign_in
    }
    fn answer(
        &self,
        token: UserPromptToken,
        answer: Secret,
        cx: &mut App,
    ) -> Result<(), TerminalError> {
        self.item()?;
        self.terminal
            .update(cx, |terminal, cx| {
                self.check(token.expected(), terminal)?;
                if token.prompt_epoch() != terminal.prompt_epoch() {
                    return Err(TerminalError::StalePrompt);
                }
                if !matches!(terminal.prompt(), Some(Prompt::Secret { .. })) {
                    return Err(TerminalError::SignInNoLongerRequested);
                }
                terminal.answer_secret_and_remember(answer, false, cx);
                Ok(())
            })
            .map_err(|_| TerminalError::Gone)?
    }
}

impl RemoteReconnect for Access {
    fn reconnect(
        &self,
        expected: ExpectedSession,
        cx: &mut App,
    ) -> Result<ExpectedSession, TerminalError> {
        self.item()?;
        self.terminal
            .update(cx, |terminal, cx| {
                self.check(expected, terminal)?;
                if terminal.is_local() || terminal.is_command() {
                    return Err(TerminalError::OnlyRemoteReconnect);
                }
                if matches!(terminal.status(), Status::Closed(_)) {
                    terminal.reconnect(cx);
                }
                Ok(ExpectedSession::new(
                    expected.item(),
                    terminal.connection_epoch(),
                ))
            })
            .map_err(|_| TerminalError::Gone)?
    }
}
