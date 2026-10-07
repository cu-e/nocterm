//! Platform event ownership and composed input. Encoding stays in nocterm-vt.
use super::*;
use gpui_kit::KeyUpEvent;
use nocterm_vt::{
    KeyEncoding, KeyEvent, KeyEventKind, KeyboardState, encode_key_event, encode_text_commit,
};

const MAX_PRESSED_KEYS: usize = 128;

#[derive(Default)]
pub(super) struct KeyboardInput {
    context: Option<(KeyboardState, bool, u64, u64)>,
    pressed: Vec<String>,
    pending_text: Option<String>,
}

impl KeyboardInput {
    pub(super) fn clear(&mut self) {
        self.pressed.clear();
        self.pending_text = None;
        self.context = None;
    }

    fn sync(&mut self, state: KeyboardState, alt_screen: bool, epoch: u64, protocol_epoch: u64) {
        let context = (state, alt_screen, epoch, protocol_epoch);
        if self.context != Some(context) {
            self.clear();
            self.context = Some(context);
        }
    }

    fn forget(&mut self, key: &str) -> bool {
        if self.pending_text.as_deref() == Some(key) {
            self.pending_text = None;
        }
        let Some(index) = self.pressed.iter().position(|owned| owned == key) else {
            return false;
        };
        self.pressed.swap_remove(index);
        true
    }

    fn own(&mut self, key: String) {
        if self.pressed.len() < MAX_PRESSED_KEYS && !self.pressed.contains(&key) {
            self.pressed.push(key);
        }
    }
}

impl TerminalView {
    pub(super) fn sync_keyboard(&mut self, cx: &App) {
        let terminal = self.terminal.read(cx);
        if !terminal.is_connected() || terminal.prompt().is_some() {
            self.keyboard.clear();
            return;
        }
        let modes = terminal.emulator().modes();
        self.keyboard.sync(
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
        self.keyboard.pending_text = None;
        if !event.is_held {
            self.keyboard.forget(key);
        }
        if !self.focus_handle.is_focused(window)
            || event.keystroke.modifiers.platform
            || self.marked_text.is_some()
            || !self.terminal.read(cx).is_connected()
        {
            self.keyboard.forget(key);
            return;
        }
        if event.prefer_character_input {
            // AltGr/layout and platform composition already consumed modifiers.
            // Their text commit is authoritative and has no reliable physical key.
            self.keyboard.forget(key);
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
                self.keyboard.pending_text = None;
                let accepted = self.type_text(
                    std::str::from_utf8(&bytes).expect("keyboard encoding is UTF-8"),
                    cx,
                );
                if accepted && modes.keyboard.report_event_types {
                    self.keyboard.own(key.to_owned());
                }
            }
            KeyEncoding::Text => {
                self.keyboard.pending_text =
                    modes.keyboard.report_event_types.then(|| key.to_owned());
            }
            KeyEncoding::Ignored => {
                self.keyboard.forget(key);
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
        let owned = self.keyboard.forget(key);
        if !owned
            || !self.focus_handle.is_focused(window)
            || event.keystroke.modifiers.platform
            || self.marked_text.is_some()
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
        let pending = self.keyboard.pending_text.take();
        let bytes = encode_text_commit(text, modes);
        if self.type_text(
            std::str::from_utf8(&bytes).expect("text encoding is UTF-8"),
            cx,
        ) && let Some(key) = pending
        {
            self.keyboard.own(key);
        }
    }

    pub(super) fn on_keyboard_binding(
        &mut self,
        event: &gpui_kit::KeystrokeEvent,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        if event.action.is_some() && self.focus_handle.is_focused(window) {
            self.keyboard.pending_text = None;
            self.keyboard.forget(&event.keystroke.key);
        }
    }
}
