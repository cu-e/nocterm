use super::*;

pub(super) fn encode(press: &KeyPress<'_>, modes: Modes) -> Option<Vec<u8>> {
    let modifiers = press.modifiers;
    let parameter = modifiers.parameter();
    let modified = parameter != 1;

    match press.key {
        "enter" => return Some(meta(modifiers.alt, b"\r")),
        "escape" => return Some(meta(modifiers.alt, b"\x1b")),
        "tab" if modifiers.shift => return Some(meta(modifiers.alt, b"\x1b[Z")),
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
pub(super) fn meta(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(bytes.len() + 1);
    if alt {
        encoded.push(0x1b);
    }
    encoded.extend_from_slice(bytes);
    encoded
}

pub(super) fn cursor_key(key: &str) -> Option<char> {
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

pub(super) fn ss3_function_key(key: &str) -> Option<char> {
    Some(match key {
        "f1" => 'P',
        "f2" => 'Q',
        "f3" => 'R',
        "f4" => 'S',
        _ => return None,
    })
}

pub(super) fn tilde_key(key: &str) -> Option<u8> {
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
pub(super) fn control_byte(ch: char) -> Option<u8> {
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
