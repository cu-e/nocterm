//! Turning key presses into the bytes a terminal program expects.
//!
//! The encodings are xterm's: `CSI` sequences for navigation keys, an
//! `ESC` prefix for alt, control characters for ctrl.

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

/// The bytes to send for a key press.
///
/// `None` means the key only types text; the host's text input delivers that
/// separately, which is what makes dead keys and input methods work.
pub fn encode_key(press: &KeyPress<'_>, modes: Modes) -> Option<Vec<u8>> {
    let modifiers = press.modifiers;
    let parameter = modifiers.parameter();
    let modified = parameter != 1;

    match press.key {
        "enter" => return Some(meta(modifiers.alt, b"\r")),
        "escape" => return Some(meta(modifiers.alt, b"\x1b")),
        "tab" if modifiers.shift => return Some(b"\x1b[Z".to_vec()),
        "tab" => return Some(meta(modifiers.alt, b"\t")),
        "backspace" => {
            let erase: &[u8] = if modifiers.ctrl { b"\x08" } else { b"\x7f" };
            return Some(meta(modifiers.alt, erase));
        }
        "space" if modifiers.ctrl => return Some(meta(modifiers.alt, b"\0")),
        "space" if modifiers.alt => return Some(b"\x1b ".to_vec()),
        "space" => return None,
        _ => {}
    }

    if let Some(letter) = cursor_key(press.key) {
        return Some(if modified {
            format!("\x1b[1;{parameter}{letter}").into_bytes()
        } else if modes.app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        });
    }

    if let Some(letter) = ss3_function_key(press.key) {
        return Some(if modified {
            format!("\x1b[1;{parameter}{letter}").into_bytes()
        } else {
            format!("\x1bO{letter}").into_bytes()
        });
    }

    if let Some(number) = tilde_key(press.key) {
        return Some(if modified {
            format!("\x1b[{number};{parameter}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        });
    }

    let mut chars = press.key.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if modifiers.ctrl {
        return control_byte(ch).map(|byte| meta(modifiers.alt, &[byte]));
    }
    if modifiers.alt {
        let typed = match press.text.filter(|text| !text.is_empty()) {
            Some(text) => text.to_owned(),
            None if modifiers.shift => press.key.to_uppercase(),
            None => press.key.to_owned(),
        };
        return Some(meta(true, typed.as_bytes()));
    }
    None
}

/// Prefixes `bytes` with `ESC` when alt (meta) is held.
fn meta(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(bytes.len() + 1);
    if alt {
        encoded.push(0x1b);
    }
    encoded.extend_from_slice(bytes);
    encoded
}

fn cursor_key(key: &str) -> Option<char> {
    Some(match key {
        "up" => 'A',
        "down" => 'B',
        "right" => 'C',
        "left" => 'D',
        "home" => 'H',
        "end" => 'F',
        _ => return None,
    })
}

fn ss3_function_key(key: &str) -> Option<char> {
    Some(match key {
        "f1" => 'P',
        "f2" => 'Q',
        "f3" => 'R',
        "f4" => 'S',
        _ => return None,
    })
}

fn tilde_key(key: &str) -> Option<u8> {
    Some(match key {
        "insert" => 2,
        "delete" => 3,
        "pageup" => 5,
        "pagedown" => 6,
        "f5" => 15,
        "f6" => 17,
        "f7" => 18,
        "f8" => 19,
        "f9" => 20,
        "f10" => 21,
        "f11" => 23,
        "f12" => 24,
        _ => return None,
    })
}

/// The control character ctrl produces with `ch`, where there is one.
fn control_byte(ch: char) -> Option<u8> {
    Some(match ch.to_ascii_lowercase() {
        letter @ 'a'..='z' => letter as u8 - b'a' + 1,
        '@' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '7' | '/' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
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
