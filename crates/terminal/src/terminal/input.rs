//! User snippet pasting and guarded agent input share transport encoding.
use crate::file_drop::FileDropError;

use super::{Terminal, TerminalEvent};
use gpui_kit::Context;
use nocterm_vt::encode_paste;
use nocterm_workspace::{LiveCommandLease, LocalDirectoryError, TerminalError};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub(super) struct AgentInput {
    lease: Option<Arc<AtomicBool>>,
    submitting: bool,
}
impl AgentInput {
    pub(super) fn revoke(&mut self) {
        if let Some(lease) = self.lease.take() {
            lease.store(false, Ordering::Release);
        }
    }
    fn active(&self) -> bool {
        self.lease
            .as_ref()
            .is_some_and(|lease| lease.load(Ordering::Acquire))
    }
}

impl Terminal {
    /// Changes a known idle, empty local prompt. Otherwise returns a prepared command.
    pub fn change_directory(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<(), LocalDirectoryError> {
        if !self.local || !self.is_connected() {
            return Err(LocalDirectoryError::NotConnectedLocal);
        }
        let _ = cx;
        let stem = Path::new(&self.shell_program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let path = path.to_str().ok_or(LocalDirectoryError::NonUtf8Path)?;
        if path.contains(['\0', '\r', '\n']) {
            return Err(LocalDirectoryError::ControlCharacters);
        }
        let command = match stem {
            "bash" | "zsh" | "fish" => format!("cd -- {}", nocterm_session::quote_posix(path)),
            "pwsh" | "powershell" => format!(
                "Set-Location -LiteralPath {}",
                nocterm_session::quote_powershell(path)
            ),
            _ => return Err(LocalDirectoryError::UnsupportedShell),
        };
        let integration = self.integration.borrow();
        if !integration.at_prompt || integration.dirty_input || self.emulator.modes().alt_screen {
            return Err(LocalDirectoryError::Busy { command });
        }
        drop(integration);
        let bytes = self.codec.encode(&format!("{command}\r"))?;
        if !self.send(bytes) {
            return Err(LocalDirectoryError::QueueFull);
        }
        Ok(())
    }
    pub(crate) fn drop_syntax(&self) -> Option<nocterm_workspace::ShellSyntax> {
        self.shell_syntax
    }

    /// Effective program for the running connection, including per-tab overrides.
    pub(crate) fn drop_shell(&self) -> &str {
        &self.shell_program
    }

    // ── Input ────────────────────────────────────────────────────────────────

    pub(crate) fn agent_prompt_state(&self) -> (Option<bool>, bool) {
        let integration = self.integration.borrow();
        (
            integration.known.then_some(integration.at_prompt),
            integration.dirty_input,
        )
    }

    pub(crate) fn agent_send(
        &mut self,
        text: &str,
        command: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), TerminalError> {
        if !self.is_connected() || self.prompt.is_some() {
            return Err(TerminalError::Unavailable);
        }
        if text.len() > 16 * 1024 {
            return Err(TerminalError::InputTooLarge);
        }
        if command {
            if text.contains(['\0', '\r', '\n']) {
                return Err(TerminalError::CommandControlCharacters);
            }
            let integration = self.integration.borrow();
            if self.emulator.modes().alt_screen
                || !integration.known
                || !integration.at_prompt
                || integration.dirty_input
            {
                return Err(TerminalError::CommandPromptUnavailable);
            }
        }
        if self.agent_input.borrow().active() && !self.agent_input.borrow().submitting {
            return Err(TerminalError::LiveCommandOwned);
        }
        let text = if command {
            format!("{text}\r")
        } else {
            text.to_owned()
        };
        let bytes = self.codec.encode(&text)?;
        self.agent_input.borrow_mut().submitting = true;
        let accepted = self.send(bytes);
        self.agent_input.borrow_mut().submitting = false;
        if !accepted {
            return Err(TerminalError::InputQueueRejected);
        }
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }

    pub(crate) fn agent_executor(&self) -> Option<Arc<dyn nocterm_session::HostExec>> {
        self.session
            .as_ref()
            .and_then(nocterm_session::Session::exec)
    }

    pub(crate) fn begin_live_command(
        &mut self,
        command: &str,
        cx: &mut Context<Self>,
    ) -> Result<LiveCommandLease, TerminalError> {
        if self.agent_input.borrow().active() {
            return Err(TerminalError::LiveCommandOwned);
        }
        self.agent_send(command, true, cx)?;
        let active = Arc::new(AtomicBool::new(true));
        self.agent_input.borrow_mut().lease = Some(active.clone());
        Ok(LiveCommandLease::new(active, self.session.clone()))
    }

    /// Delivers user bytes and revokes any live agent ownership on accepted input.
    pub fn send(&self, bytes: impl Into<Vec<u8>>) -> bool {
        self.send_bytes(bytes.into(), true)
    }
    pub(crate) fn send_protocol(&self, bytes: impl Into<Vec<u8>>) -> bool {
        self.send_bytes(bytes.into(), false)
    }
    fn send_bytes(&self, bytes: Vec<u8>, user: bool) -> bool {
        if let (Some(session), super::Status::Connected) = (&self.session, &self.status) {
            let accepted = session.input(bytes.clone());
            if accepted && user {
                self.integration.borrow_mut().input(&bytes);
                let mut input = self.agent_input.borrow_mut();
                if !input.submitting {
                    input.revoke();
                }
            }
            *self.input_error.borrow_mut() = if accepted {
                None
            } else {
                Some("Input was not sent: the connection's input queue is full or the paste exceeds 4 MiB. Wait and retry with a smaller selection.".into())
            };
            accepted
        } else {
            false
        }
    }

    /// Encodes only user text. Protocol replies and mouse messages use send unchanged.
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let previous = self.text_error();
        let accepted = match self.codec.encode(text) {
            Ok(bytes) => {
                self.text_error = None;
                self.send(bytes)
            }
            Err(error) => {
                self.text_error = Some(error.to_string());
                false
            }
        };
        if previous != self.text_error() {
            cx.emit(TerminalEvent::Changed);
        }
        accepted
    }

    /// Paste a user-selected snippet in one transport submission. Shell integration
    /// is optional: Windows shells and SSH sessions follow the normal paste protocol.
    pub(crate) fn paste_snippet(
        &mut self,
        text: &str,
        execute: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), TerminalError> {
        if !self.is_connected() || self.prompt.is_some() || self.awaiting_vault() {
            return Err(TerminalError::Unavailable);
        }
        if self.emulator.modes().alt_screen {
            return Err(TerminalError::SnippetAlternateScreen);
        }
        if text.trim().is_empty() {
            return Err(TerminalError::EmptySnippet);
        }
        if text.len() > 256 * 1024 {
            return Err(TerminalError::SnippetTooLarge);
        }
        if text.contains(['\0', '\x1b']) {
            return Err(TerminalError::SnippetControlCharacters);
        }
        let text = if execute {
            text.trim_end_matches(['\r', '\n'])
        } else {
            text
        };
        self.send_paste(text, execute, cx)
    }

    /// Explorer drops follow native paste, including programs in the alternate screen.
    pub(crate) fn paste_paths(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), FileDropError> {
        if !self.is_connected() || self.prompt.is_some() || self.awaiting_vault() {
            return Err(FileDropError::NotConnected);
        }
        if text.is_empty() || text.len() > 256 * 1024 || text.chars().any(char::is_control) {
            return Err(FileDropError::UnsafeSelection);
        }
        self.send_paste(text, false, cx)
            .map_err(FileDropError::from)
    }

    fn send_paste(
        &mut self,
        text: &str,
        execute: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), TerminalError> {
        let mut paste = encode_paste(text, self.emulator.modes());
        // Enter must follow the bracketed-paste closing marker, never occur inside it.
        if execute {
            paste.push(b'\r');
        }
        let text = std::str::from_utf8(&paste)?;
        let bytes = self.codec.encode(text)?;
        if !self.send(bytes) {
            return Err(TerminalError::PasteQueueRejected);
        }
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }
}
#[cfg(test)]
mod tests;
