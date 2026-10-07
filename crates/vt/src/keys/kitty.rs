//! Kitty progressive enhancements. Legacy text remains with the host IME
//! unless the application explicitly requests reporting every key.
use super::*;

pub(super) fn encode(event: &KeyEvent<'_>, modes: Modes) -> KeyEncoding {
    let state = modes.keyboard;
    let press = &event.press;
    let editing = matches!(press.key, "enter" | "tab" | "backspace");
    let modified = press.modifiers.parameter() != 1;
    let text = character(press.key).is_some();
    let modifier_key = matches!(
        press.key,
        "shift"
            | "ctrl"
            | "control"
            | "alt"
            | "super"
            | "meta"
            | "leftshift"
            | "rightshift"
            | "leftctrl"
            | "rightctrl"
            | "leftalt"
            | "rightalt"
    );
    if event.kind == KeyEventKind::Release
        && (!state.report_event_types || (editing && !state.report_all_keys))
    {
        return KeyEncoding::Ignored;
    }
    if modifier_key && !state.report_all_keys {
        return KeyEncoding::Ignored;
    }
    let disambiguated = state.disambiguate
        && (press.key == "escape"
            || press.modifiers.ctrl
            || press.modifiers.alt
            || (editing && modified));
    let functional = !editing && !text && !modifier_key;
    let enhanced = state.disambiguate || state.report_event_types || state.report_all_keys;
    let escaped = state.report_all_keys
        || disambiguated
        || (functional && enhanced)
        || event.kind == KeyEventKind::Release;
    if !escaped {
        return match legacy::encode(press, modes) {
            Some(bytes) => KeyEncoding::Encoded(bytes),
            None if text_key(press) => KeyEncoding::Text,
            None => KeyEncoding::Ignored,
        };
    }
    let Some((code, trailer)) = code(press.key) else {
        return KeyEncoding::Ignored;
    };
    let mut payload = code.to_string();
    if state.report_alternate_keys && trailer == 'u' {
        let shifted = event
            .shifted_key
            .filter(|ch| press.modifiers.shift && !ch.is_control());
        if let Some(shifted) = shifted {
            payload.push_str(&format!(":{}", shifted as u32));
        }
        if let Some(base) = event.base_layout_key.filter(|ch| !ch.is_control()) {
            if shifted.is_none() {
                payload.push(':');
            }
            payload.push_str(&format!(":{}", base as u32));
        }
    }
    let associated = press.text.filter(|text| {
        state.report_all_keys
            && state.report_associated_text
            && event.kind != KeyEventKind::Release
            && !text.is_empty()
            && !text.chars().any(char::is_control)
    });
    let event_type = state.report_event_types && event.kind != KeyEventKind::Press;
    let parameter = press.modifiers.parameter();
    let mut bytes = String::from("\x1b[");
    // Cursor and F1/F2/F4 omit their default first parameter when unmodified.
    if code != 1
        || trailer == 'u'
        || trailer == '~'
        || parameter != 1
        || event_type
        || associated.is_some()
    {
        bytes.push_str(&payload);
    }
    if parameter != 1 || event_type || associated.is_some() {
        bytes.push_str(&format!(";{parameter}"));
    }
    if event_type {
        bytes.push_str(if event.kind == KeyEventKind::Repeat {
            ":2"
        } else {
            ":3"
        });
    }
    if let Some(text) = associated {
        append_text(&mut bytes, text);
    }
    bytes.push(trailer);
    KeyEncoding::Encoded(bytes.into_bytes())
}

pub(super) fn text_commit(text: &str, state: KeyboardState) -> Vec<u8> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut bytes = String::from("\x1b[0");
    if state.report_associated_text && !text.chars().any(char::is_control) {
        bytes.push(';');
        append_text(&mut bytes, text);
    }
    bytes.push('u');
    bytes.into_bytes()
}

fn append_text(bytes: &mut String, text: &str) {
    bytes.push(';');
    for (index, ch) in text.chars().enumerate() {
        if index != 0 {
            bytes.push(':');
        }
        bytes.push_str(&(ch as u32).to_string());
    }
}

fn code(key: &str) -> Option<(u32, char)> {
    if let Some(ch) = character(key) {
        return Some((ch as u32, 'u'));
    }
    if let Some(letter) = legacy::cursor_key(key) {
        return Some((1, letter));
    }
    if let Some(number) = legacy::tilde_key(key) {
        return Some((u32::from(number), '~'));
    }
    Some(match key {
        "enter" => (13, 'u'),
        "tab" => (9, 'u'),
        "backspace" => (127, 'u'),
        "escape" => (27, 'u'),
        "f1" => (1, 'P'),
        "f2" => (1, 'Q'),
        "f3" => (13, '~'),
        "f4" => (1, 'S'),
        "capslock" => (57358, 'u'),
        "scrolllock" => (57359, 'u'),
        "numlock" => (57360, 'u'),
        "printscreen" => (57361, 'u'),
        "pause" => (57362, 'u'),
        "menu" => (57363, 'u'),
        "leftshift" => (57441, 'u'),
        "rightshift" => (57447, 'u'),
        "leftctrl" => (57442, 'u'),
        "rightctrl" => (57448, 'u'),
        "leftalt" => (57443, 'u'),
        "rightalt" => (57449, 'u'),
        _ => {
            let number = key.strip_prefix('f')?.parse::<u32>().ok()?;
            if !(13..=35).contains(&number) {
                return None;
            }
            (57376 + number - 13, 'u')
        }
    })
}
