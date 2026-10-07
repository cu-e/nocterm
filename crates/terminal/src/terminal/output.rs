//! Decode and advance session output, publishing semantic mode transitions.
use super::{Terminal, TerminalEvent};
use gpui_kit::Context;

impl Terminal {
    pub(super) fn advance_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let bytes = self.codec.decode(bytes, false);
        if let Some(recording) = &mut self.recording {
            recording.output(&bytes);
        }
        if self.integration.borrow_mut().advance(&bytes) {
            cx.emit(TerminalEvent::Changed);
        }
        let alt_screen = self.emulator.modes().alt_screen;
        let effects = self.emulator.advance(&bytes);
        self.apply(effects, cx);
        if self.emulator.modes().alt_screen != alt_screen {
            // Panels cache command availability. Screen output by itself is
            // visual; entering or leaving a full-screen program changes it.
            cx.emit(TerminalEvent::Changed);
        }
        self.schedule_sync(cx);
        self.refresh_find(cx);
        cx.emit(TerminalEvent::Output);
    }
}
