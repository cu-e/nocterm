use super::paste::{dropped_paths, quote_paths_with_syntax};
use super::*;
use crate::file_drop::FileDropError;
use gpui_kit::KeyUpEvent;
use nocterm_vt::{
    KeyEncoding, KeyEvent, KeyEventKind, KeyboardState, encode_key_event, encode_text_commit,
};
use nocterm_workspace::FileDrag;

#[derive(Default)]
pub(super) struct ViewInput {
    keyboard: keyboard::KeyboardInput,
    marked_text: Option<String>,
    focused: bool,
}
impl ViewInput {
    pub(super) fn focused(&self) -> bool {
        self.focused
    }
    pub(super) fn focus_changed(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.keyboard.clear();
        }
    }
    pub(super) fn composing(&self) -> bool {
        self.marked_text.is_some()
    }
    pub(super) fn composition(&self) -> Option<&str> {
        self.marked_text.as_deref()
    }
    pub(super) fn clear_composition(&mut self) -> bool {
        self.marked_text.take().is_some()
    }
    pub(super) fn mark(&mut self, text: &str) -> bool {
        self.keyboard.clear();
        let marked = Some(text.to_owned()).filter(|text| !text.is_empty());
        if self.marked_text == marked {
            return false;
        }
        self.marked_text = marked;
        true
    }
    fn clear_keys(&mut self) {
        self.keyboard.clear();
    }
    fn sync(&mut self, state: KeyboardState, alt_screen: bool, epoch: u64, protocol_epoch: u64) {
        self.keyboard.sync(state, alt_screen, epoch, protocol_epoch);
    }
    fn clear_pending(&mut self) {
        self.keyboard.clear_pending();
    }
    fn forget_key(&mut self, key: &str) -> bool {
        self.keyboard.forget(key)
    }
    fn accept_key(&mut self, key: String) {
        self.keyboard.own(key);
    }
    fn defer(&mut self, key: Option<String>) {
        self.keyboard.defer(key);
    }
    fn take_pending(&mut self) -> Option<String> {
        self.keyboard.take_pending()
    }
}

/// Text input: what the keyboard types once keys are composed, and what an
/// input method is composing. The terminal has no editable text of its own,
/// so ranges are relative to the composition.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .input
            .composition()
            .map_or(0, |text| text.encode_utf16().count());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.input
            .composition()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.input.clear_composition() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let composing = self.input.clear_composition();
        if !text.is_empty() {
            self.commit_text(text, cx);
        }
        if composing {
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input.mark(text) {
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let geometry = self.painted_geometry()?;
        let cursor = self.cache.frame().cursor?;
        Some(geometry.cell_bounds(
            CellPoint {
                row: cursor.row,
                col: cursor.col,
            },
            1,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl TerminalView {
    pub(super) fn sync_keyboard(&mut self, cx: &App) {
        let terminal = self.terminal.read(cx);
        if !terminal.is_connected() || terminal.prompt().is_some() {
            self.input.clear_keys();
            return;
        }
        let modes = terminal.emulator().modes();
        self.input.sync(
            modes.keyboard,
            modes.alt_screen,
            terminal.prompt_epoch(),
            modes.keyboard_protocol_epoch,
        );
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_keyboard(cx);
        let key = &event.keystroke.key;
        self.input.clear_pending();
        if !event.is_held {
            self.input.forget_key(key);
        }
        if !self.focus_handle.is_focused(window)
            || event.keystroke.modifiers.platform
            || self.input.composing()
            || !self.terminal.read(cx).is_connected()
        {
            self.input.forget_key(key);
            return;
        }
        if event.prefer_character_input {
            // AltGr/layout and platform composition already consumed modifiers.
            // Their text commit is authoritative and has no reliable physical key.
            self.input.forget_key(key);
            return;
        }
        let press = KeyPress {
            key,
            modifiers: modifiers(&event.keystroke.modifiers),
            text: event.keystroke.key_char.as_deref(),
        };
        let kind = if event.is_held {
            KeyEventKind::Repeat
        } else {
            KeyEventKind::Press
        };
        let mut key_event = KeyEvent::new(press, kind);
        key_event.shifted_key = press.text.and_then(|text| {
            let mut chars = text.chars();
            let ch = chars.next()?;
            (chars.next().is_none() && !ch.is_control()).then_some(ch)
        });
        let modes = self.terminal.read(cx).emulator().modes();
        match encode_key_event(&key_event, modes) {
            KeyEncoding::Encoded(bytes) => {
                cx.stop_propagation();
                self.input.clear_pending();
                let accepted = self.type_text(
                    std::str::from_utf8(&bytes).expect("keyboard encoding is UTF-8"),
                    cx,
                );
                if accepted && modes.keyboard.report_event_types {
                    self.input.accept_key(key.to_owned());
                }
            }
            KeyEncoding::Text => {
                self.input
                    .defer(modes.keyboard.report_event_types.then(|| key.to_owned()));
            }
            KeyEncoding::Ignored => {
                self.input.forget_key(key);
            }
        }
    }

    pub(super) fn on_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_keyboard(cx);
        let key = &event.keystroke.key;
        let owned = self.input.forget_key(key);
        if !owned
            || !self.focus_handle.is_focused(window)
            || event.keystroke.modifiers.platform
            || self.input.composing()
        {
            return;
        }
        let press = KeyPress {
            key,
            modifiers: modifiers(&event.keystroke.modifiers),
            text: None,
        };
        let modes = self.terminal.read(cx).emulator().modes();
        if let KeyEncoding::Encoded(bytes) =
            encode_key_event(&KeyEvent::new(press, KeyEventKind::Release), modes)
        {
            cx.stop_propagation();
            // Releases never constitute editing: keep viewport and selection.
            self.terminal.read(cx).send(bytes);
        }
    }

    pub(super) fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.sync_keyboard(cx);
        if text.is_empty() || !self.terminal.read(cx).is_connected() {
            return;
        }
        let modes = self.terminal.read(cx).emulator().modes();
        let pending = self.input.take_pending();
        let bytes = encode_text_commit(text, modes);
        if self.type_text(
            std::str::from_utf8(&bytes).expect("text encoding is UTF-8"),
            cx,
        ) && let Some(key) = pending
        {
            self.input.accept_key(key);
        }
    }

    pub(super) fn on_keyboard_binding(
        &mut self,
        event: &gpui_kit::KeystrokeEvent,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        if event.action.is_some() && self.focus_handle.is_focused(window) {
            self.input.clear_pending();
            self.input.forget_key(&event.keystroke.key);
        }
    }
}

impl TerminalView {
    pub(super) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let modes = self.terminal.read(cx).emulator().modes();
        self.type_text(
            std::str::from_utf8(&encode_paste(text, modes))
                .expect("paste wrapping preserves UTF-8"),
            cx,
        );
    }

    pub(super) fn drop_files(
        &mut self,
        files: &FileDrag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let terminal = self.terminal.read(cx);
        let result =
            if terminal.is_connected() && terminal.prompt().is_none() && !terminal.awaiting_vault()
            {
                dropped_paths(files).and_then(|paths| {
                    quote_paths_with_syntax(
                        &paths,
                        terminal.drop_shell(),
                        terminal.is_local(),
                        terminal.drop_syntax(),
                    )
                })
            } else {
                Err(FileDropError::NotConnected)
            };
        match result {
            Ok(text) => {
                let result = self.terminal.update(cx, |terminal, cx| {
                    terminal.paste_paths(&text, cx)?;
                    terminal.complete_native_paste(cx);
                    Ok::<_, FileDropError>(())
                });
                if let Err(error) = result {
                    nocterm_ui::notice::warning(
                        window,
                        cx,
                        "terminal-file-drop",
                        "Could not insert file paths",
                        error.to_string(),
                    );
                } else {
                    window.focus(&self.focus_handle, cx);
                    self.wake_cursor(cx);
                }
            }
            Err(error) => nocterm_ui::notice::warning(
                window,
                cx,
                "terminal-file-drop",
                "Could not insert file paths",
                error.to_string(),
            ),
        }
    }
}
