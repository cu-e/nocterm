//! User snippet pasting and guarded agent input share transport encoding.
use super::{Terminal, TerminalEvent};
use gpui_kit::Context;
use nocterm_vt::encode_paste;

impl Terminal {
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
        let mut paste = encode_paste(text, self.emulator.modes());
        // Enter must follow the bracketed-paste closing marker, never occur inside it.
        if execute {
            paste.push(b'\r');
        }
        let text =
            std::str::from_utf8(&paste).map_err(|_| "Snippet could not be encoded as text.")?;
        let bytes = self.codec.encode(text)?;
        if !self.send(bytes) {
            return Err("Terminal input queue rejected the snippet. Wait and retry.".into());
        }
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }
}
#[cfg(test)]
mod tests;
