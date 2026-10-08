use super::*;

fn state(flags: u8) -> KeyboardState {
    KeyboardState {
        disambiguate: flags & 1 != 0,
        report_event_types: flags & 2 != 0,
        report_alternate_keys: flags & 4 != 0,
        report_all_keys: flags & 8 != 0,
        report_associated_text: flags & 16 != 0,
        ..Default::default()
    }
}

fn event(key: &str, modifiers: Modifiers, kind: KeyEventKind, flags: u8) -> KeyEncoding {
    encode_key_event(
        &KeyEvent::new(
            KeyPress {
                key,
                modifiers,
                text: None,
            },
            kind,
        ),
        Modes {
            keyboard: state(flags),
            ..Default::default()
        },
    )
}

#[test]
fn codex_flags_seven_disambiguate_enter_without_changing_plain_enter() {
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    assert_eq!(
        event("enter", shift, KeyEventKind::Press, 7),
        KeyEncoding::Encoded(b"\x1b[13;2u".to_vec())
    );
    assert_eq!(
        event("enter", Default::default(), KeyEventKind::Press, 7),
        KeyEncoding::Encoded(b"\r".to_vec())
    );
    assert_eq!(
        event("enter", shift, KeyEventKind::Release, 7),
        KeyEncoding::Ignored
    );
    assert_eq!(
        event("enter", shift, KeyEventKind::Press, 0),
        KeyEncoding::Encoded(b"\r".to_vec())
    );
}

#[test]
fn editing_and_control_key_matrix_uses_negotiated_encodings() {
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let ctrl = Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    for (key, modifiers, expected) in [
        ("enter", shift, "\x1b[13;2u"),
        ("enter", ctrl, "\x1b[13;5u"),
        ("tab", shift, "\x1b[9;2u"),
        ("tab", alt, "\x1b[9;3u"),
        ("backspace", ctrl, "\x1b[127;5u"),
        ("escape", Default::default(), "\x1b[27u"),
        ("c", ctrl, "\x1b[99;5u"),
        ("ё", ctrl, "\x1b[1105;5u"),
        ("a", alt, "\x1b[97;3u"),
        ("f3", shift, "\x1b[13;2~"),
        ("up", ctrl, "\x1b[1;5A"),
        ("f13", ctrl, "\x1b[57376;5u"),
    ] {
        assert_eq!(
            event(key, modifiers, KeyEventKind::Press, 7),
            KeyEncoding::Encoded(expected.as_bytes().to_vec()),
            "{key}"
        );
    }
    for key in ["enter", "tab", "backspace"] {
        assert_eq!(
            event(key, ctrl, KeyEventKind::Release, 7),
            KeyEncoding::Ignored
        );
    }
}

#[test]
fn independent_flags_preserve_legacy_text_and_report_only_requested_events() {
    let none = Modifiers::default();
    let ctrl = Modifiers { ctrl: true, ..none };
    assert_eq!(event("a", none, KeyEventKind::Press, 1), KeyEncoding::Text);
    assert_eq!(event("a", none, KeyEventKind::Repeat, 2), KeyEncoding::Text);
    assert_eq!(
        event("a", none, KeyEventKind::Release, 2),
        KeyEncoding::Encoded(b"\x1b[97;1:3u".to_vec())
    );
    assert_eq!(
        event("c", ctrl, KeyEventKind::Press, 4),
        KeyEncoding::Encoded(b"\x03".to_vec())
    );
    assert_eq!(event("a", none, KeyEventKind::Press, 16), KeyEncoding::Text);
    assert_eq!(
        event("a", none, KeyEventKind::Press, 8),
        KeyEncoding::Encoded(b"\x1b[97u".to_vec())
    );
    assert_eq!(
        event("up", none, KeyEventKind::Repeat, 2),
        KeyEncoding::Encoded(b"\x1b[1;1:2A".to_vec())
    );
    assert_eq!(
        event("up", none, KeyEventKind::Release, 1),
        KeyEncoding::Ignored
    );
    assert_eq!(
        event("enter", none, KeyEventKind::Release, 10),
        KeyEncoding::Encoded(b"\x1b[13;1:3u".to_vec())
    );
    assert_eq!(
        event("f3", none, KeyEventKind::Press, 1),
        KeyEncoding::Encoded(b"\x1b[13~".to_vec())
    );
}

#[test]
fn alternate_keys_use_only_supplied_values_and_associated_text_never_contains_controls() {
    let key_event = KeyEvent {
        press: KeyPress {
            key: "я",
            modifiers: Modifiers {
                shift: true,
                ctrl: true,
                ..Default::default()
            },
            text: Some("Я"),
        },
        kind: KeyEventKind::Press,
        shifted_key: Some('Я'),
        base_layout_key: Some('z'),
    };
    assert_eq!(
        encode_key_event(
            &key_event,
            Modes {
                keyboard: state(31),
                ..Default::default()
            }
        ),
        KeyEncoding::Encoded(b"\x1b[1103:1071:122;6;1071u".to_vec())
    );
    let key_event = KeyEvent {
        press: KeyPress {
            text: Some("\x03"),
            ..key_event.press
        },
        ..key_event
    };
    assert_eq!(
        encode_key_event(
            &key_event,
            Modes {
                keyboard: state(31),
                ..Default::default()
            }
        ),
        KeyEncoding::Encoded(b"\x1b[1103:1071:122;6u".to_vec())
    );
    assert_eq!(
        encode_text_commit(
            "日e\u{301}",
            Modes {
                keyboard: state(24),
                ..Default::default()
            }
        ),
        b"\x1b[0;;26085:101:769u"
    );
    assert_eq!(
        encode_text_commit(
            "日",
            Modes {
                keyboard: state(8),
                ..Default::default()
            }
        ),
        b"\x1b[0u"
    );
    assert_eq!(
        encode_text_commit(
            "日",
            Modes {
                keyboard: state(16),
                ..Default::default()
            }
        ),
        "日".as_bytes()
    );
    assert_eq!(
        event("shift", Default::default(), KeyEventKind::Press, 31),
        KeyEncoding::Ignored
    );
}

#[test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn xterm_levels_preserve_level_one_exceptions_and_encode_level_two_modified_text() {
    for (mode, key, modifiers, text, expected) in [
        (
            ModifyOtherKeys::ExceptWellDefined,
            "c",
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
            None,
            "\x03",
        ),
        (
            ModifyOtherKeys::ExceptWellDefined,
            "tab",
            Modifiers {
                shift: true,
                ..Default::default()
            },
            None,
            "\x1b[Z",
        ),
        (
            ModifyOtherKeys::ExceptWellDefined,
            "tab",
            Modifiers {
                alt: true,
                ..Default::default()
            },
            None,
            "\x1b[27;3;9~",
        ),
        (
            ModifyOtherKeys::All,
            "enter",
            Modifiers {
                shift: true,
                ..Default::default()
            },
            None,
            "\x1b[27;2;13~",
        ),
        (
            ModifyOtherKeys::All,
            "tab",
            Modifiers {
                shift: true,
                ..Default::default()
            },
            None,
            "\x1b[Z",
        ),
        (
            ModifyOtherKeys::All,
            "a",
            Modifiers {
                shift: true,
                ..Default::default()
            },
            Some("A"),
            "\x1b[27;2;65~",
        ),
        (
            ModifyOtherKeys::All,
            "ё",
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
            None,
            "\x1b[27;5;1105~",
        ),
    ] {
        let modes = Modes {
            keyboard: KeyboardState {
                modify_other_keys: mode,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            encode_key(
                &KeyPress {
                    key,
                    modifiers,
                    text
                },
                modes
            )
            .unwrap(),
            expected.as_bytes()
        );
    }
    let modes = Modes {
        keyboard: KeyboardState {
            modify_other_keys: ModifyOtherKeys::All,
            ..state(1)
        },
        ..Default::default()
    };
    assert_eq!(
        encode_key(
            &KeyPress {
                key: "enter",
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
                text: None
            },
            modes
        )
        .unwrap(),
        b"\x1b[13;2u"
    );
}

#[test]
fn xterm_level_one_keeps_control_space_as_a_well_defined_alias() {
    let modes = Modes {
        keyboard: KeyboardState {
            modify_other_keys: ModifyOtherKeys::ExceptWellDefined,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        encode_key(
            &KeyPress {
                key: "space",
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                text: None
            },
            modes
        )
        .unwrap(),
        b"\0"
    );
}

#[test]
fn legacy_alt_shift_tab_preserves_meta_modifier() {
    assert_eq!(
        encode_key(
            &KeyPress {
                key: "tab",
                text: None,
                modifiers: Modifiers {
                    alt: true,
                    shift: true,
                    ..Default::default()
                },
            },
            Modes::default()
        )
        .unwrap(),
        b"\x1b\x1b[Z",
    );
}

#[test]
fn xterm_uses_logical_key_when_platform_text_is_a_control_character() {
    assert_eq!(
        encode_key(
            &KeyPress {
                key: "a",
                text: Some("\x01"),
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            Modes {
                keyboard: KeyboardState {
                    modify_other_keys: ModifyOtherKeys::All,
                    ..Default::default()
                },
                ..Default::default()
            }
        )
        .unwrap(),
        b"\x1b[27;5;97~",
    );
}

#[test]
fn kitty_rejects_control_scalars_in_optional_alternate_identities() {
    let mut key = KeyEvent::new(
        KeyPress {
            key: "a",
            text: Some("\x01"),
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        },
        KeyEventKind::Press,
    );
    key.shifted_key = Some('\x01');
    key.base_layout_key = Some('\n');
    assert_eq!(
        encode_key_event(
            &key,
            Modes {
                keyboard: state(7),
                ..Default::default()
            }
        ),
        KeyEncoding::Encoded(b"\x1b[97;6u".to_vec())
    );
}
