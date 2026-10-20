//! User snippet pasting and guarded agent input share transport encoding.
use crate::file_drop::FileDropError;

use super::{Terminal, TerminalEvent};
use crate::credentials::credential_for;
use gpui_kit::{App, Context};
use nocterm_session::{HostKeyDecision, Prompt, Secret, SecretRequest};
use nocterm_vt::encode_paste;
use nocterm_workspace::{LiveCommandLease, LocalDirectoryError, TerminalError};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{codec::TextCodec, integration::ShellIntegration};
use nocterm_session::{Charset, TextEncodingError};
use nocterm_vt::Modes;
use std::cell::RefCell;

#[derive(Clone, Copy)]
enum SubmissionPolicy {
    User,
    Agent,
    Protocol,
}
struct PreparedInput {
    bytes: Vec<u8>,
    policy: SubmissionPolicy,
}

#[derive(Default)]
struct AgentInput {
    lease: Option<Arc<AtomicBool>>,
}
impl AgentInput {
    fn revoke(&mut self) {
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

pub(super) struct TerminalInput {
    codec: TextCodec,
    text_error: Option<String>,
    input_error: RefCell<Option<String>>,
    integration: RefCell<ShellIntegration>,
    agent: RefCell<AgentInput>,
    shell_program: String,
    shell_syntax: Option<nocterm_workspace::ShellSyntax>,
}
impl TerminalInput {
    pub(super) fn new(charset: Charset) -> Self {
        Self {
            codec: TextCodec::new(charset),
            text_error: None,
            input_error: RefCell::default(),
            integration: RefCell::default(),
            agent: RefCell::default(),
            shell_program: String::new(),
            shell_syntax: None,
        }
    }
    pub(super) fn reset_charset(&mut self, charset: Charset) {
        self.codec = TextCodec::new(charset);
    }
    pub(super) fn clear_errors(&mut self) {
        self.text_error = None;
        *self.input_error.borrow_mut() = None;
    }
    pub(super) fn set_shell(&mut self, program: String, local: bool) {
        self.shell_program = program;
        if local {
            *self.integration.borrow_mut() = ShellIntegration::default();
        }
    }
    pub(super) fn set_shell_syntax(&mut self, syntax: Option<nocterm_workspace::ShellSyntax>) {
        self.shell_syntax = syntax;
    }
    pub(super) fn revoke(&self) {
        self.agent.borrow_mut().revoke();
        *self.integration.borrow_mut() = ShellIntegration::default();
    }
    pub(super) fn decode(&mut self, bytes: &[u8], last: bool) -> Vec<u8> {
        self.codec.decode(bytes, last)
    }
    pub(super) fn advance_integration(&self, bytes: &[u8]) -> bool {
        self.integration.borrow_mut().advance(bytes)
    }
    pub(super) fn cwd(&self) -> Option<std::path::PathBuf> {
        self.integration.borrow().cwd().map(Path::to_path_buf)
    }
    pub(super) fn text_error(&self) -> Option<String> {
        self.text_error
            .clone()
            .or_else(|| self.input_error.borrow().clone())
    }
    pub(super) fn set_text_error(&mut self, error: Option<String>) {
        self.text_error = error;
    }
    fn shell_syntax(&self) -> Option<nocterm_workspace::ShellSyntax> {
        self.shell_syntax
    }
    fn shell_program(&self) -> &str {
        &self.shell_program
    }
    fn live_active(&self) -> bool {
        self.agent.borrow().active()
    }
    fn prompt_state(&self) -> (Option<bool>, bool) {
        let integration = self.integration.borrow();
        integration.prompt_state()
    }
    fn prepare_directory(
        &self,
        path: &Path,
        alt_screen: bool,
    ) -> Result<PreparedInput, LocalDirectoryError> {
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
        if !integration.at_prompt() || integration.dirty_input() || alt_screen {
            return Err(LocalDirectoryError::Busy { command });
        }
        Ok(PreparedInput {
            bytes: self.codec.encode(&format!("{command}\r"))?,
            policy: SubmissionPolicy::User,
        })
    }
    fn prepare_agent(
        &self,
        text: &str,
        command: bool,
        alt_screen: bool,
    ) -> Result<PreparedInput, TerminalError> {
        if text.len() > 16 * 1024 {
            return Err(TerminalError::InputTooLarge);
        }
        if command {
            if text.contains(['\0', '\r', '\n']) {
                return Err(TerminalError::CommandControlCharacters);
            }
            let integration = self.integration.borrow();
            if alt_screen
                || !integration.known()
                || !integration.at_prompt()
                || integration.dirty_input()
            {
                return Err(TerminalError::CommandPromptUnavailable);
            }
        }
        if self.agent.borrow().active() {
            return Err(TerminalError::LiveCommandOwned);
        }
        let text = if command {
            format!("{text}\r")
        } else {
            text.to_owned()
        };
        Ok(PreparedInput {
            bytes: self.codec.encode(&text)?,
            policy: SubmissionPolicy::Agent,
        })
    }
    fn prepare_paste(
        &self,
        text: &str,
        execute: bool,
        modes: Modes,
    ) -> Result<PreparedInput, TerminalError> {
        let mut paste = encode_paste(text, modes);
        if execute {
            paste.push(b'\r');
        }
        let text = std::str::from_utf8(&paste)?;
        Ok(PreparedInput {
            bytes: self.codec.encode(text)?,
            policy: SubmissionPolicy::User,
        })
    }
    fn prepare_text(&self, text: &str) -> Result<PreparedInput, TextEncodingError> {
        Ok(PreparedInput {
            bytes: self.codec.encode(text)?,
            policy: SubmissionPolicy::User,
        })
    }
    fn commit_submission(&self, prepared: &PreparedInput, accepted: bool) {
        if accepted && !matches!(prepared.policy, SubmissionPolicy::Protocol) {
            self.integration.borrow_mut().input(&prepared.bytes);
            if matches!(prepared.policy, SubmissionPolicy::User) {
                self.agent.borrow_mut().revoke();
            }
        }
        *self.input_error.borrow_mut() = if accepted {
            None
        } else {
            Some("Input was not sent: the connection's input queue is full or the paste exceeds 4 MiB. Wait and retry with a smaller selection.".into())
        };
    }
    fn commit_live(&self, session: Option<nocterm_session::Session>) -> LiveCommandLease {
        let active = Arc::new(AtomicBool::new(true));
        self.agent.borrow_mut().lease = Some(active.clone());
        LiveCommandLease::new(active, session)
    }
}

impl Terminal {
    /// Changes a known idle, empty local prompt. Otherwise returns a prepared command.
    pub fn change_directory(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<(), LocalDirectoryError> {
        if !self.is_local() || !self.is_connected() {
            return Err(LocalDirectoryError::NotConnectedLocal);
        }
        let _ = cx;
        let prepared = self
            .input
            .prepare_directory(path, self.emulator().modes().alt_screen)?;
        if !self.submit_prepared(prepared) {
            return Err(LocalDirectoryError::QueueFull);
        }
        Ok(())
    }
    pub(crate) fn drop_syntax(&self) -> Option<nocterm_workspace::ShellSyntax> {
        self.input.shell_syntax()
    }

    /// Effective program for the running connection, including per-tab overrides.
    pub(crate) fn drop_shell(&self) -> &str {
        self.input.shell_program()
    }

    // ── Input ────────────────────────────────────────────────────────────────

    pub(crate) fn agent_prompt_state(&self) -> (Option<bool>, bool) {
        self.input.prompt_state()
    }

    pub(crate) fn agent_send(
        &mut self,
        text: &str,
        command: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), TerminalError> {
        if !self.is_connected() || self.prompt().is_some() {
            return Err(TerminalError::Unavailable);
        }
        let prepared =
            self.input
                .prepare_agent(text, command, self.emulator().modes().alt_screen)?;
        if !self.submit_prepared(prepared) {
            return Err(TerminalError::InputQueueRejected);
        }
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }

    pub(crate) fn agent_executor(&self) -> Option<Arc<dyn nocterm_session::HostExec>> {
        self.lifecycle
            .session()
            .and_then(nocterm_session::Session::exec)
    }

    pub(crate) fn begin_live_command(
        &mut self,
        command: &str,
        cx: &mut Context<Self>,
    ) -> Result<LiveCommandLease, TerminalError> {
        if self.input.live_active() {
            return Err(TerminalError::LiveCommandOwned);
        }
        self.agent_send(command, true, cx)?;
        Ok(self.input.commit_live(self.lifecycle.session().cloned()))
    }

    /// Delivers user bytes and revokes any live agent ownership on accepted input.
    pub fn send(&self, bytes: impl Into<Vec<u8>>) -> bool {
        self.send_bytes(bytes.into(), true)
    }
    pub(crate) fn send_protocol(&self, bytes: impl Into<Vec<u8>>) -> bool {
        self.send_bytes(bytes.into(), false)
    }
    fn send_bytes(&self, bytes: Vec<u8>, user: bool) -> bool {
        let policy = if user {
            SubmissionPolicy::User
        } else {
            SubmissionPolicy::Protocol
        };
        self.submit_prepared(PreparedInput { bytes, policy })
    }
    fn submit_prepared(&self, prepared: PreparedInput) -> bool {
        if !self.is_connected() {
            return false;
        }
        let Some(session) = self.lifecycle.session() else {
            return false;
        };
        let accepted = session.input(prepared.bytes.clone());
        self.input.commit_submission(&prepared, accepted);
        accepted
    }

    /// Encodes only user text. Protocol replies and mouse messages use send unchanged.
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let previous = self.text_error();
        let accepted = match self.input.prepare_text(text) {
            Ok(prepared) => {
                self.input.set_text_error(None);
                self.submit_prepared(prepared)
            }
            Err(error) => {
                self.input.set_text_error(Some(error.to_string()));
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
        if !self.is_connected() || self.prompt().is_some() || self.awaiting_vault() {
            return Err(TerminalError::Unavailable);
        }
        if self.emulator().modes().alt_screen {
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
        if !self.is_connected() || self.prompt().is_some() || self.awaiting_vault() {
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
        let prepared = self
            .input
            .prepare_paste(text, execute, self.emulator().modes())?;
        if !self.submit_prepared(prepared) {
            return Err(TerminalError::PasteQueueRejected);
        }
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }
}
impl Terminal {
    pub(crate) fn clear_credentials(&mut self) {
        self.credentials.clear();
    }
    pub fn vault_unlocked(&self, cx: &App) -> bool {
        let _ = cx;
        self.deps
            .credentials()
            .is_some_and(|provider| provider.store().is_available())
    }
    /// Whether unlocking the vault would answer the current prompt.
    pub fn awaiting_vault(&self) -> bool {
        self.credentials.awaiting_vault() && self.prompt().is_some()
    }
    pub fn credential_message(&self) -> Option<&str> {
        self.credentials.message()
    }
    pub fn clear_credential_message(&mut self, cx: &mut Context<Self>) {
        self.credentials.set_message(None);
        cx.emit(TerminalEvent::Changed);
    }
    pub(crate) fn retrieve_credential(&mut self, cx: &mut Context<Self>) {
        self.credentials.begin_lookup();
        let Some(Prompt::Secret { request, hop, .. }) = self.prompt() else {
            return;
        };
        let request = request.clone();
        let credential = credential_for(self.spec(), hop.as_ref());
        if !matches!(request, SecretRequest::Interactive { .. }) {
            self.credentials.cancel_candidate();
        }
        if matches!(
            request,
            SecretRequest::Password { retry: true, .. }
                | SecretRequest::KeyPassphrase { retry: true, .. }
        ) {
            return;
        }
        if !request.is_storable() {
            return;
        }
        let Some(id) = credential else {
            return;
        };
        let Some(provider) = self.deps.credentials() else {
            return;
        };
        if !provider.store().is_available() {
            self.credentials.set_message(Some(
                "Unlock the credential vault to use the saved secret, or enter it here.".into(),
            ));
            self.credentials.wait_for_vault(true);
            let available = provider.store().wait_available();
            self.wait_for_unlock(available, cx);
            return;
        }
        let future = provider.store().get(id, &request);
        let epoch = self.credentials.epoch();
        self.credentials
            .install_lookup(cx.spawn(async move |this, cx| {
                let result = future.await;
                let _ = this.update(cx, |this, cx| {
                    if !this.credentials.matches(epoch) || this.prompt().is_none() {
                        return;
                    }
                    match result {
                        Ok(secret) => this.answer_secret(Some(secret), cx),
                        Err(error) => {
                            this.credentials.set_message(Some(format!(
                                "Saved credential unavailable: {error}"
                            )));
                            cx.emit(TerminalEvent::Changed);
                        }
                    }
                });
            }));
    }
    /// Answers the prompt with the saved secret as soon as the vault is
    /// unlocked, whether from the prompt's link, Settings or device unlock.
    fn wait_for_unlock(
        &mut self,
        available: impl Future<Output = ()> + 'static,
        cx: &mut Context<Self>,
    ) {
        let epoch = self.credentials.epoch();
        self.credentials
            .install_lookup(cx.spawn(async move |this, cx| {
                available.await;
                let _ = this.update(cx, |this, cx| {
                    if this.credentials.matches(epoch) && this.prompt().is_some() {
                        this.credentials.wait_for_vault(false);
                        this.retrieve_credential(cx);
                        cx.emit(TerminalEvent::Changed);
                    }
                });
            }));
    }
    /// Remember is an explicit user choice. It never applies to interactive/MFA
    /// or to a jump host, whose secret cannot be bound to the destination profile.
    pub fn answer_secret_and_remember(
        &mut self,
        secret: Secret,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        self.credentials.invalidate();
        let interactive = matches!(
            self.prompt(),
            Some(Prompt::Secret {
                request: SecretRequest::Interactive { .. },
                ..
            })
        );
        if !interactive {
            self.credentials.cancel_candidate();
        }
        if remember
            && self.vault_unlocked(cx)
            && let Some(Prompt::Secret {
                request, hop: None, ..
            }) = self.prompt()
            && request.is_storable()
        {
            self.credentials
                .remember_candidate(request.clone(), secret.clone());
        }
        self.answer_secret(Some(secret), cx);
    }
    pub(crate) fn cancel_credential_candidate(&mut self) {
        self.credentials.cancel_candidate();
    }
    pub(crate) fn save_authenticated_credential(&mut self, cx: &mut Context<Self>) {
        let Some((request, secret)) = self.credentials.take_candidate() else {
            return;
        };
        let Some(provider) = self.deps.credentials() else {
            return;
        };
        let saved = provider.association();
        let spec = self.spec().clone();
        // Association is published only after the credential is saved.
        let future = provider
            .store()
            .save(spec.title.to_string(), &request, secret);
        let epoch = self.credentials.epoch();
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let association = this.update(cx, |this, cx| {
                if !this.credentials.matches(epoch) {
                    return None;
                }
                match result {
                    Ok(id) => {
                        this.set_credential_id(id);
                        return Some(saved(&spec, id, cx));
                    }
                    Err(error) => this
                        .credentials
                        .set_message(Some(format!("Could not save credential: {error}"))),
                }
                cx.emit(TerminalEvent::Changed);
                None
            });
            if let Ok(Some(association)) = association {
                let result = association.await;
                let _ = this.update(cx, |this, cx| {
                    if !this.credentials.matches(epoch) {
                        return;
                    }
                    this.credentials.set_message(Some(match result {
                        Ok(()) => "Credential saved in the encrypted vault.".into(),
                        Err(error) => format!(
                            "Credential saved; could not link it to connection history: {error}"
                        ),
                    }));
                    cx.emit(TerminalEvent::Changed);
                });
            }
        })
        .detach();
    }
}

impl Terminal {
    // ── Prompts ──────────────────────────────────────────────────────────────

    /// Answers a pending host key question.
    pub fn answer_host_key(&mut self, decision: HostKeyDecision, cx: &mut Context<Self>) {
        self.lifecycle.answer_host_key(decision);
        cx.emit(TerminalEvent::Changed);
    }

    /// Answers a pending request for a secret; `None` declines it.
    pub fn answer_secret(&mut self, secret: Option<Secret>, cx: &mut Context<Self>) {
        if secret.is_none() {
            self.cancel_credential_candidate();
        }
        self.lifecycle.answer_secret(secret);
        cx.emit(TerminalEvent::Changed);
    }
}

#[cfg(test)]
mod tests;
