//! Turning key presses into the bytes a terminal program expects.
//!
//! The negotiated protocol decides whether a key is legacy input, a kitty
//! event, an xterm modifyOtherKeys event, or text delivered by the IME.

use crate::Modes;

/// Modifier keys held during a key press or a mouse event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    /// xterm's modifier parameter: 1 for none, plus 1, 2 and 4 for shift, alt and ctrl.
    fn parameter(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

/// A key press, described the way the host toolkit reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress<'a> {
    /// Lower-case name of the key: the character printed on it (`"a"`, `"/"`)
    /// or a name (`"enter"`, `"up"`, `"pageup"`, `"f5"`).
    pub key: &'a str,
    pub modifiers: Modifiers,
    /// What the key types with its modifiers applied (`"A"` for shift-a).
    pub text: Option<&'a str>,
}

mod kitty;
mod legacy;
#[cfg(test)]
mod protocol_tests;
mod xterm;
#[cfg(test)]
mod xterm_compat_tests;

/// xterm's negotiated encoding level for ordinary modified keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModifyOtherKeys {
    #[default]
    Off,
    ExceptWellDefined,
    All,
}

/// Progressive keyboard features negotiated by the running application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardState {
    pub disambiguate: bool,
    pub report_event_types: bool,
    pub report_alternate_keys: bool,
    pub report_all_keys: bool,
    pub report_associated_text: bool,
    pub modify_other_keys: ModifyOtherKeys,
}

impl KeyboardState {
    pub fn kitty_flags(self) -> u8 {
        u8::from(self.disambiguate)
            | u8::from(self.report_event_types) << 1
            | u8::from(self.report_alternate_keys) << 2
            | u8::from(self.report_all_keys) << 3
            | u8::from(self.report_associated_text) << 4
    }
}

/// Physical key event type, distinct from an IME text commit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyEventKind {
    #[default]
    Press,
    Repeat,
    Release,
}

/// Host key information. Optional alternate values must come from the host;
/// the encoder never guesses the physical key's base keyboard layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent<'a> {
    pub press: KeyPress<'a>,
    pub kind: KeyEventKind,
    pub shifted_key: Option<char>,
    pub base_layout_key: Option<char>,
}

impl<'a> KeyEvent<'a> {
    pub fn new(press: KeyPress<'a>, kind: KeyEventKind) -> Self {
        Self {
            press,
            kind,
            shifted_key: None,
            base_layout_key: None,
        }
    }
}

/// A key is encoded once, left to composed text input, or deliberately ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyEncoding {
    Encoded(Vec<u8>),
    Text,
    Ignored,
}

/// Encode according to negotiated keyboard features, with kitty taking priority.
pub fn encode_key_event(event: &KeyEvent<'_>, modes: Modes) -> KeyEncoding {
    if modes.keyboard.kitty_flags() != 0 {
        return kitty::encode(event, modes);
    }
    if event.kind == KeyEventKind::Release {
        return KeyEncoding::Ignored;
    }
    if let Some(bytes) = xterm::encode(&event.press, modes.keyboard.modify_other_keys) {
        return KeyEncoding::Encoded(bytes);
    }
    match legacy::encode(&event.press, modes) {
        Some(bytes) => KeyEncoding::Encoded(bytes),
        None if text_key(&event.press) => KeyEncoding::Text,
        None => KeyEncoding::Ignored,
    }
}

/// Compatibility convenience for a press. Event-aware hosts use [`encode_key_event`].
pub fn encode_key(press: &KeyPress<'_>, modes: Modes) -> Option<Vec<u8>> {
    match encode_key_event(&KeyEvent::new(*press, KeyEventKind::Press), modes) {
        KeyEncoding::Encoded(bytes) => Some(bytes),
        KeyEncoding::Text | KeyEncoding::Ignored => None,
    }
}

/// Encode a text-only commit after composition. With report-all requested this
/// uses the protocol's unknown-key code 0, and optionally includes associated text.
pub fn encode_text_commit(text: &str, modes: Modes) -> Vec<u8> {
    if modes.keyboard.report_all_keys {
        kitty::text_commit(text, modes.keyboard)
    } else {
        text.as_bytes().to_vec()
    }
}

fn character(key: &str) -> Option<char> {
    if key == "space" {
        return Some(' ');
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

fn text_key(press: &KeyPress<'_>) -> bool {
    character(press.key).is_some() || press.text.is_some_and(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        ctrl: false,
        alt: false,
        shift: false,
    };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
    const ALT: Modifiers = Modifiers { alt: true, ..NONE };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..NONE
    };

    fn encode(key: &str, modifiers: Modifiers) -> Option<Vec<u8>> {
        encode_key(
            &KeyPress {
                key,
                modifiers,
                text: None,
            },
            Modes::default(),
        )
    }

    #[track_caller]
    fn assert_encodes(key: &str, modifiers: Modifiers, expected: &[u8]) {
        assert_eq!(encode(key, modifiers).as_deref(), Some(expected), "{key}");
    }

    #[test]
    fn typing_is_left_to_text_input() {
        assert_eq!(encode("a", NONE), None);
        assert_eq!(encode("a", SHIFT), None);
        assert_eq!(encode("space", NONE), None);
        assert_eq!(encode("ё", NONE), None);
    }

    #[test]
    fn editing_keys() {
        assert_encodes("enter", NONE, b"\r");
        assert_encodes("enter", ALT, b"\x1b\r");
        assert_encodes("tab", NONE, b"\t");
        assert_encodes("tab", SHIFT, b"\x1b[Z");
        assert_encodes("escape", NONE, b"\x1b");
        assert_encodes("backspace", NONE, b"\x7f");
        assert_encodes("backspace", CTRL, b"\x08");
        assert_encodes("backspace", ALT, b"\x1b\x7f");
        assert_encodes("delete", NONE, b"\x1b[3~");
    }

    #[test]
    fn cursor_keys_follow_the_application_mode() {
        assert_encodes("up", NONE, b"\x1b[A");
        assert_encodes("home", NONE, b"\x1b[H");

        let application = Modes {
            app_cursor: true,
            ..Modes::default()
        };
        let press = |modifiers| KeyPress {
            key: "up",
            modifiers,
            text: None,
        };
        assert_eq!(encode_key(&press(NONE), application).unwrap(), b"\x1bOA");
        // A modifier always selects the CSI form, whatever the mode.
        assert_eq!(encode_key(&press(CTRL), application).unwrap(), b"\x1b[1;5A");
    }

    #[test]
    fn modifiers_are_encoded_as_a_parameter() {
        assert_encodes("left", SHIFT, b"\x1b[1;2D");
        assert_encodes("left", ALT, b"\x1b[1;3D");
        assert_encodes("right", CTRL, b"\x1b[1;5C");
        assert_encodes(
            "end",
            Modifiers {
                ctrl: true,
                alt: true,
                shift: true,
            },
            b"\x1b[1;8F",
        );
        assert_encodes("pageup", CTRL, b"\x1b[5;5~");
        assert_encodes("f1", SHIFT, b"\x1b[1;2P");
        assert_encodes("f5", CTRL, b"\x1b[15;5~");
    }

    #[test]
    fn function_keys() {
        assert_encodes("f1", NONE, b"\x1bOP");
        assert_encodes("f4", NONE, b"\x1bOS");
        assert_encodes("f5", NONE, b"\x1b[15~");
        assert_encodes("f12", NONE, b"\x1b[24~");
        assert_encodes("pagedown", NONE, b"\x1b[6~");
        assert_encodes("insert", NONE, b"\x1b[2~");
    }

    #[test]
    fn ctrl_produces_control_characters() {
        assert_encodes("c", CTRL, b"\x03");
        assert_encodes("z", CTRL, b"\x1a");
        assert_encodes("[", CTRL, b"\x1b");
        assert_encodes("space", CTRL, b"\0");
        assert_encodes("/", CTRL, b"\x1f");
        assert_encodes(
            "c",
            Modifiers {
                ctrl: true,
                alt: true,
                shift: false,
            },
            b"\x1b\x03",
        );
        assert_eq!(encode("1", CTRL), None);
    }

    #[test]
    fn alt_prefixes_what_the_key_types() {
        assert_encodes("b", ALT, b"\x1bb");
        assert_encodes(
            "b",
            Modifiers {
                alt: true,
                shift: true,
                ctrl: false,
            },
            b"\x1bB",
        );
        assert_encodes("space", ALT, b"\x1b ");

        let typed = encode_key(
            &KeyPress {
                key: ".",
                modifiers: Modifiers {
                    alt: true,
                    shift: true,
                    ctrl: false,
                },
                text: Some(">"),
            },
            Modes::default(),
        );
        assert_eq!(typed.unwrap(), b"\x1b>");
    }

    #[test]
    fn unknown_named_keys_are_ignored() {
        assert_eq!(encode("capslock", NONE), None);
        assert_eq!(encode("f13", CTRL), None);
    }
}
