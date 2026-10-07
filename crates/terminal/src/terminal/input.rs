//! User snippet pasting and guarded agent input share transport encoding.
use super::{Terminal, TerminalEvent};
use gpui_kit::Context;
use nocterm_vt::encode_paste;
use std::path::Path;

impl Terminal {
    /// Changes a known idle, empty local prompt. Otherwise returns a prepared command.
    pub fn change_directory(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.local || !self.is_connected() {
            return Err("Open a connected local terminal first.".into());
        }
        let _ = cx;
        let stem = Path::new(&self.shell_program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let path = path
            .to_str()
            .ok_or("The shell cannot represent this directory path.")?;
        if path.contains(['\0', '\r', '\n']) {
            return Err(
                "Directory paths with control characters require manual navigation.".into(),
            );
        }
        let command=match stem {
            "bash"|"zsh"|"fish"=>format!("cd -- {}",nocterm_session::quote_posix(path)),
            "pwsh"|"powershell"=>format!("Set-Location -LiteralPath {}",nocterm_session::quote_powershell(path)),
            _=>return Err("Directory synchronization needs Bash, Zsh, fish or PowerShell with shell integration enabled.".into()),
        };
        let integration = self.integration.borrow();
        if !integration.at_prompt || integration.dirty_input || self.emulator.modes().alt_screen {
            return Err(format!(
                "The shell is busy or has unfinished input. Run at an empty prompt: {command}"
            ));
        }
        drop(integration);
        let bytes = self.codec.encode(&format!("{command}\r"))?;
        if !self.send(bytes) {
            return Err(
                "Directory change was not sent because the terminal input queue is full.".into(),
            );
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
            self.local.then_some(integration.at_prompt),
            integration.dirty_input,
        )
    }

    pub(crate) fn agent_send(
        &mut self,
        text: &str,
        command: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.is_connected() || self.prompt.is_some() {
            return Err("Terminal is unavailable or waiting for authentication.".into());
        }
        if text.len() > 16 * 1024 {
            return Err("Terminal input exceeds 16 KiB.".into());
        }
        if command {
            if text.contains(['\0', '\r', '\n']) {
                return Err("Run one command without control characters.".into());
            }
            let integration = self.integration.borrow();
            if self.emulator.modes().alt_screen
                || (self.local && (!integration.at_prompt || integration.dirty_input))
            {
                return Err(
                    "The terminal is busy, has unfinished input, or is in an alternate screen."
                        .into(),
                );
            }
        }
        let text = if command {
            format!("{text}\r")
        } else {
            text.to_owned()
        };
        let bytes = self.codec.encode(&text)?;
        if !self.send(bytes) {
            return Err("Terminal input queue rejected the input.".into());
        }
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }

    /// Encodes only user text. Protocol replies and mouse messages use send unchanged.
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let previous = self.text_error();
        match self.codec.encode(text) {
            Ok(bytes) => {
                self.text_error = None;
                self.send(bytes);
            }
            Err(error) => self.text_error = Some(error),
        }
        if previous != self.text_error() {
            cx.emit(TerminalEvent::Changed);
        }
    }

    /// Paste a user-selected snippet in one transport submission. Shell integration
    /// is optional: Windows shells and SSH sessions follow the normal paste protocol.
    pub(crate) fn paste_snippet(
        &mut self,
        text: &str,
        execute: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.is_connected() || self.prompt.is_some() || self.awaiting_vault() {
            return Err("Terminal is unavailable or waiting for authentication.".into());
        }
        if self.emulator.modes().alt_screen {
            return Err("Leave the alternate screen before running a snippet.".into());
        }
        if text.trim().is_empty() {
            return Err("Snippet content is empty.".into());
        }
        if text.len() > 256 * 1024 {
            return Err("Snippet content exceeds 256 KiB.".into());
        }
        if text.contains(['\0', '\x1b']) {
            return Err("Snippet content contains NUL or Escape control characters.".into());
        }
        let text = if execute {
            text.trim_end_matches(['\r', '\n'])
        } else {
            text
        };
        self.send_paste(text, execute, cx)
    }

    /// Explorer drops follow native paste, including programs in the alternate screen.
    pub(crate) fn paste_paths(&mut self, text: &str, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.is_connected() || self.prompt.is_some() || self.awaiting_vault() {
            return Err("Connect and finish authentication before dropping paths.".into());
        }
        if text.is_empty() || text.len() > 256 * 1024 || text.chars().any(char::is_control) {
            return Err("The selected paths cannot be inserted safely.".into());
        }
        self.send_paste(text, false, cx)
    }

    fn send_paste(
        &mut self,
        text: &str,
        execute: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut paste = encode_paste(text, self.emulator.modes());
        // Enter must follow the bracketed-paste closing marker, never occur inside it.
        if execute {
            paste.push(b'\r');
        }
        let text =
            std::str::from_utf8(&paste).map_err(|_| "Paste could not be encoded as text.")?;
        let bytes = self.codec.encode(text)?;
        if !self.send(bytes) {
            return Err("Terminal input queue rejected the paste. Wait and retry.".into());
        }
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }
}
#[cfg(test)]
mod tests;
