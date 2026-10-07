//! Compatibility cases from xterm's generated US pc105 keyboard table.
use super::*;

const SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: false,
    alt: false,
};
const CTRL: Modifiers = Modifiers {
    ctrl: true,
    shift: false,
    alt: false,
};
const ALT: Modifiers = Modifiers {
    alt: true,
    ctrl: false,
    shift: false,
};

fn encoded(
    key: &str,
    modifiers: Modifiers,
    text: Option<&str>,
    level: ModifyOtherKeys,
) -> KeyEncoding {
    encode_key_event(
        &KeyEvent::new(
            KeyPress {
                key,
                modifiers,
                text,
            },
            KeyEventKind::Press,
        ),
        Modes {
            keyboard: KeyboardState {
                modify_other_keys: level,
                ..Default::default()
            },
            ..Default::default()
        },
    )
}

#[test]
fn modified_enter_and_control_tab_are_extended_at_both_levels() {
    for level in [ModifyOtherKeys::ExceptWellDefined, ModifyOtherKeys::All] {
        for (key, modifiers, expected) in [
            ("enter", SHIFT, "\x1b[27;2;13~"),
            ("enter", CTRL, "\x1b[27;5;13~"),
            ("enter", ALT, "\x1b[27;3;13~"),
            ("tab", CTRL, "\x1b[27;5;9~"),
        ] {
            assert_eq!(
                encoded(key, modifiers, None, level),
                KeyEncoding::Encoded(expected.into()),
                "{level:?} {key}"
            );
        }
        assert_eq!(
            encoded("tab", SHIFT, None, level),
            KeyEncoding::Encoded(b"\x1b[Z".to_vec())
        );
    }
}

#[test]
fn backspace_and_escape_keep_their_level_specific_exceptions() {
    let level_one = ModifyOtherKeys::ExceptWellDefined;
    let level_two = ModifyOtherKeys::All;
    for (modifiers, expected) in [(SHIFT, "\x7f"), (CTRL, "\x08"), (ALT, "\x1b\x7f")] {
        assert_eq!(
            encoded("backspace", modifiers, None, level_one),
            KeyEncoding::Encoded(expected.into())
        );
    }
    assert_eq!(
        encoded("backspace", CTRL, None, level_two),
        KeyEncoding::Encoded(b"\x08".to_vec())
    );
    assert_eq!(
        encoded("backspace", SHIFT, None, level_two),
        KeyEncoding::Encoded(b"\x1b[27;2;127~".to_vec())
    );
    assert_eq!(
        encoded("backspace", ALT, None, level_two),
        KeyEncoding::Encoded(b"\x1b[27;3;127~".to_vec())
    );
    for modifiers in [SHIFT, CTRL] {
        assert_eq!(
            encoded("escape", modifiers, None, level_one),
            KeyEncoding::Encoded(b"\x1b".to_vec())
        );
    }
    assert_eq!(
        encoded("escape", ALT, None, level_one),
        KeyEncoding::Encoded(b"\x1b[27;3;27~".to_vec())
    );
    assert_eq!(
        encoded("escape", SHIFT, None, level_two),
        KeyEncoding::Encoded(b"\x1b[27;2;27~".to_vec())
    );
    assert_eq!(
        encoded("escape", CTRL, None, level_two),
        KeyEncoding::Encoded(b"\x1b[27;5;27~".to_vec())
    );
}

#[test]
fn shift_only_printable_input_respects_xterm_control_input_range() {
    for (key, text) in [("1", "!"), (".", ">"), ("/", "?")] {
        assert_eq!(
            encoded(key, SHIFT, Some(text), ModifyOtherKeys::All),
            KeyEncoding::Text,
            "{text}"
        );
    }
    for (key, text, codepoint) in [
        ("a", "A", 65),
        ("\\", "|", 124),
        ("]", "}", 125),
        ("`", "~", 126),
    ] {
        assert_eq!(
            encoded(key, SHIFT, Some(text), ModifyOtherKeys::All),
            KeyEncoding::Encoded(format!("\x1b[27;2;{codepoint}~").into_bytes())
        );
    }
    assert_eq!(
        encoded(
            "space",
            SHIFT,
            Some(" "),
            ModifyOtherKeys::ExceptWellDefined
        ),
        KeyEncoding::Text
    );
    assert_eq!(
        encoded("space", SHIFT, Some(" "), ModifyOtherKeys::All),
        KeyEncoding::Encoded(b"\x1b[27;2;32~".to_vec())
    );
}

#[test]
fn legacy_alt_shift_tab_and_control_text_are_safe_without_negotiation() {
    let alt_shift = Modifiers { alt: true, ..SHIFT };
    assert_eq!(
        encoded("tab", alt_shift, None, ModifyOtherKeys::Off),
        KeyEncoding::Encoded(b"\x1b\x1b[Z".to_vec())
    );
    for level in [ModifyOtherKeys::ExceptWellDefined, ModifyOtherKeys::All] {
        assert_eq!(
            encoded("a", SHIFT, Some("\x01"), level),
            if level == ModifyOtherKeys::All {
                KeyEncoding::Encoded(b"\x1b[27;2;65~".to_vec())
            } else {
                KeyEncoding::Text
            }
        );
    }
    assert_eq!(
        encoded("c", CTRL, Some("\x03"), ModifyOtherKeys::ExceptWellDefined),
        KeyEncoding::Encoded(b"\x03".to_vec())
    );
}
