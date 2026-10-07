use super::*;

impl Terminal {
    /// Tracks presentation mutations before event subscribers run.
    pub(crate) fn display_revision(&self) -> u64 {
        self.display_revision
    }

    pub(super) fn emit_output(&mut self, cx: &mut Context<Self>) {
        self.display_revision = self.display_revision.wrapping_add(1);
        cx.emit(TerminalEvent::Output);
    }

    /// Gives the emulator to `edit`, for selection and scrolling, and redraws.
    pub fn update_emulator<R>(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Emulator) -> R,
    ) -> R {
        let result = edit(&mut self.emulator);
        self.emit_output(cx);
        result
    }

    pub fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        if self.emulator.scroll(scroll) {
            self.emit_output(cx);
        }
    }

    /// Returns to live output and removes selection only when input changes the screen.
    pub(crate) fn prepare_input(&mut self, cx: &mut Context<Self>) {
        if self.emulator.prepare_input() {
            self.emit_output(cx);
        }
    }
}
