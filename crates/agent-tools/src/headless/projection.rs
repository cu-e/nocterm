//! Actual VT text/prompt state with incremental session decoding.
use super::HeadlessError;
use nocterm_cloud_model::{authority::RunBinding, identity::IntentId, tool::ScopedToolRequest};
use nocterm_session::{Charset, PtySize};
use nocterm_vt::{
    Emulator, EmulatorOptions, ShellIntegration, TermSize, TextCodec, TextQuery, TextTail,
};
use std::sync::Arc;

pub(super) struct TerminalProjection {
    emulator: Emulator,
    codec: TextCodec,
    integration: ShellIntegration,
    live: LiveOwnership,
    charset: Charset,
}
impl TerminalProjection {
    pub(super) fn new(size: PtySize, charset: Charset) -> Self {
        Self {
            emulator: Emulator::new(
                TermSize::new(size.cols, size.rows, 0, 0),
                EmulatorOptions {
                    scrollback_lines: 2000,
                    ..EmulatorOptions::default()
                },
            ),
            codec: TextCodec::new(charset),
            integration: ShellIntegration::default(),
            live: LiveOwnership::default(),
            charset,
        }
    }
    /// Titles, bell and clipboard requests have no headless side effects.
    /// Protocol replies stay bytes and are admitted by the session owner.
    pub(super) fn output(&mut self, bytes: &[u8], last: bool, now: u64) -> Vec<Vec<u8>> {
        let decoded = self.codec.decode(bytes, last);
        self.integration.advance(&decoded);
        let effects = self.emulator.advance_at(&decoded, now);
        if self.at_prompt() {
            self.live.prompt_returned();
        }
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                nocterm_vt::Effect::Reply(bytes) => Some(bytes),
                _ => None,
            })
            .collect()
    }
    /// Restore is a picture, never authority that a live shell is idle.
    pub(super) fn restore(&mut self, bytes: &[u8], now: u64) {
        self.emulator.reset();
        self.integration = ShellIntegration::default();
        self.codec = TextCodec::new(self.charset);
        let decoded = self.codec.decode(bytes, false);
        let _ = self.emulator.advance_at(&decoded, now);
    }
    pub(super) fn text(&self, lines: usize, since: Option<u64>) -> TextTail {
        self.emulator.text(TextQuery {
            max_lines: lines,
            max_bytes: nocterm_ai::tools::MAX_READ_BYTES,
            since_line: since,
        })
    }
    pub(super) fn prepare_input(
        &self,
        text: &str,
        command: bool,
    ) -> Result<Vec<u8>, HeadlessError> {
        if text.len() > nocterm_ai::tools::MAX_INPUT_BYTES {
            return Err(HeadlessError::InputTooLarge);
        }
        if self.live.current.is_some() {
            return Err(HeadlessError::LiveCommandOwned);
        }
        if command
            && (self.emulator.modes().alt_screen
                || !self.integration.known()
                || !self.integration.at_prompt()
                || self.integration.dirty_input())
        {
            return Err(HeadlessError::CommandPromptUnavailable);
        }
        let text = if command {
            format!("{text}\r")
        } else {
            text.to_owned()
        };
        Ok(self.codec.encode(&text)?)
    }
    pub(super) fn input_admitted(&mut self, bytes: &[u8], owner: Option<LiveOwner>) {
        self.integration.input(bytes);
        self.live.current = owner;
    }
    pub(super) fn generation(&self) -> u64 {
        self.emulator.generation()
    }
    pub(super) fn end_live(&mut self, owner: &LiveOwner) {
        if self
            .live
            .current
            .as_ref()
            .is_some_and(|current| current.same_as(owner))
        {
            self.live.current = None;
        }
    }
    pub(super) fn prompt_returned(&self, owner: &LiveOwner) -> bool {
        self.live
            .completed
            .as_ref()
            .is_some_and(|completed| completed.same_as(owner))
    }

    pub(super) fn at_prompt(&self) -> bool {
        self.integration.known()
            && self.integration.at_prompt()
            && !self.integration.dirty_input()
            && !self.emulator.modes().alt_screen
    }
}

#[derive(Default)]
struct LiveOwnership {
    current: Option<LiveOwner>,
    completed: Option<LiveOwner>,
}
impl LiveOwnership {
    fn prompt_returned(&mut self) {
        if let Some(owner) = self.current.take() {
            self.completed = Some(owner);
        }
    }
}

/// Each physical acquisition has its own local identity even if all supplied
/// coordinates repeat. Run/intent remain descriptive metadata, not a lease.
#[derive(Clone)]
pub(super) struct LiveOwner {
    run: RunBinding,
    intent: IntentId,
    generation: Arc<()>,
}
impl LiveOwner {
    fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.generation, &other.generation)
            && self.run == other.run
            && self.intent == other.intent
    }
    pub(super) fn new(request: &ScopedToolRequest) -> Self {
        Self {
            run: request.permit().intent().authority().run().clone(),
            intent: request.permit().intent().id(),
            generation: Arc::new(()),
        }
    }
}
