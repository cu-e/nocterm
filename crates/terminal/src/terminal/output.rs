//! Decode and advance session output, publishing semantic mode transitions.
use super::{Terminal, TerminalEvent};
use gpui_kit::Context;

impl Terminal {
    pub(super) fn apply_session_batch(
        &mut self,
        epoch: u64,
        batch: Vec<nocterm_session::Event>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.lifecycle.is_current(epoch) {
            return false;
        }
        for event in batch {
            self.handle_event(event, cx);
        }
        true
    }

    pub(super) fn advance_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let bytes = self.input.decode(bytes, false);
        self.recording.output(&bytes, cx);
        if self.input.advance_integration(&bytes) {
            cx.emit(TerminalEvent::Changed);
        }
        let alt_screen = self.emulator().modes().alt_screen;
        let effects = self.screen.advance(&bytes);
        self.apply(effects, cx);
        if self.emulator().modes().alt_screen != alt_screen {
            // Panels cache command availability. Screen output by itself is
            // visual; entering or leaving a full-screen program changes it.
            cx.emit(TerminalEvent::Changed);
        }
        self.schedule_sync(cx);
        self.refresh_find(cx);
        self.emit_output(cx);
    }
    pub(crate) fn display_revision(&self) -> u64 {
        self.screen.revision()
    }
    pub(super) fn emit_output(&mut self, cx: &mut Context<Self>) {
        self.screen.emit_output(cx);
    }
    pub fn scroll(&mut self, scroll: nocterm_vt::Scroll, cx: &mut Context<Self>) {
        self.screen.scroll(scroll, cx);
    }
    pub(crate) fn prepare_input(&mut self, cx: &mut Context<Self>) {
        self.screen.prepare_input(cx);
    }
    pub(crate) fn complete_native_paste(&mut self, cx: &mut Context<Self>) {
        self.screen.complete_native_paste(cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.screen.select_all(cx);
    }
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.screen.clear_selection(cx);
    }
    pub fn begin_selection(
        &mut self,
        kind: nocterm_vt::SelectionKind,
        at: nocterm_vt::CellPoint,
        side: nocterm_vt::Side,
        cx: &mut Context<Self>,
    ) {
        self.screen.begin_selection(kind, at, side, cx);
    }
    pub fn extend_selection(
        &mut self,
        at: nocterm_vt::CellPoint,
        side: nocterm_vt::Side,
        scroll: i32,
        cx: &mut Context<Self>,
    ) -> bool {
        self.screen.extend_selection(at, side, scroll, cx)
    }
    pub fn find(&self) -> &super::FindState {
        self.screen.find()
    }
    pub fn begin_find(
        &mut self,
        query: String,
        direction: nocterm_vt::SearchDirection,
        anchor: Option<nocterm_vt::SearchPoint>,
        cx: &mut Context<Self>,
    ) {
        self.screen.begin_find(query, direction, anchor, cx);
    }
    pub fn set_find_options(&mut self, options: nocterm_vt::SearchOptions, cx: &mut Context<Self>) {
        self.screen.set_find_options(options, cx);
    }
    pub fn find_next(&mut self, previous: bool, cx: &mut Context<Self>) {
        self.screen.find_next(previous, cx);
    }
    pub fn clear_find(&mut self, cx: &mut Context<Self>) {
        self.screen.clear_find(cx);
    }
    pub(super) fn refresh_find(&mut self, cx: &mut Context<Self>) {
        self.screen.refresh_find(cx);
    }
}
