use super::*;

fn emulator() -> Emulator {
    Emulator::new(TermSize::new(80, 3, 8, 16), EmulatorOptions::default())
}
fn replies(effects: Vec<Effect>) -> Vec<Vec<u8>> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Reply(bytes) => Some(bytes),
            _ => None,
        })
        .collect()
}

#[test]
fn detection_negotiation_direct_set_union_difference_and_nested_restore() {
    let mut terminal = emulator();
    let probe = replies(terminal.advance(b"\x1b[?u\x1b[c"));
    assert_eq!(probe[0], b"\x1b[?0u");
    assert!(probe[1].starts_with(b"\x1b[?"));
    assert!(probe[1].ends_with(b"c"));
    for (sequence, flags) in [
        (b"\x1b[=5u".as_slice(), 5),
        (b"\x1b[=2;2u", 7),
        (b"\x1b[=4;3u", 3),
        (b"\x1b[>7u", 7),
        (b"\x1b[>31u", 31),
        (b"\x1b[<u", 7),
        (b"\x1b[<u", 0),
    ] {
        terminal.advance(sequence);
        assert_eq!(terminal.modes().keyboard.kitty_flags(), flags);
        assert_eq!(
            replies(terminal.advance(b"\x1b[?u")),
            [format!("\x1b[?{flags}u").into_bytes()]
        );
    }
    terminal.advance(b"\x1b[>1u\x1b[>2u\x1b[<999u");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 0);
    for flag in [1, 2, 4, 8, 16] {
        terminal.advance(format!("\x1b[={flag}u").as_bytes());
        assert_eq!(terminal.modes().keyboard.kitty_flags(), flag);
    }
}

#[test]
fn main_and_alternate_keep_separate_current_flags_and_saved_stacks() {
    let mut terminal = emulator();
    terminal.advance(b"\x1b[=5u\x1b[>7u\x1b[?1049h");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 0);
    terminal.advance(b"\x1b[=2u\x1b[>31u\x1b[?1049l");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 7);
    terminal.advance(b"\x1b[>1u\x1b[<u");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 7);
    terminal.advance(b"\x1b[?1049h");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 31);
    terminal.advance(b"\x1b[<u");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 0);
    terminal.advance(b"\x1bc\x1b[?1049h");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 0);
}

#[test]
fn stack_overflow_evicts_oldest_keyboard_state_without_touching_titles_or_panicking() {
    let mut terminal = emulator();
    for number in 0..4100 {
        terminal.advance(format!("\x1b[>{}u", number % 32).as_bytes());
    }
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 3);
    terminal.advance(b"\x1b[<1u");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 2);
    terminal.advance(b"\x1b[<65535u");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 0);
}

#[test]
fn xterm_modifier_resource_is_global_queries_restore_and_reset_disables_it() {
    let mut terminal = emulator();
    for (level, expected) in [
        (1, crate::ModifyOtherKeys::ExceptWellDefined),
        (2, crate::ModifyOtherKeys::All),
        (0, crate::ModifyOtherKeys::Off),
    ] {
        terminal.advance(format!("\x1b[>4;{level}m").as_bytes());
        assert_eq!(terminal.modes().keyboard.modify_other_keys, expected);
        assert_eq!(
            replies(terminal.advance(b"\x1b[?4m")),
            [format!("\x1b[>4;{level}m").into_bytes()]
        );
    }
    terminal.advance(b"\x1b[>4;2m\x1b[?1049h");
    assert_eq!(
        terminal.modes().keyboard.modify_other_keys,
        crate::ModifyOtherKeys::All
    );
    terminal.advance(b"\x1b[?1049l\x1bc");
    assert_eq!(
        terminal.modes().keyboard.modify_other_keys,
        crate::ModifyOtherKeys::Off
    );
}

#[test]
fn keyboard_epoch_tracks_negotiations_even_when_final_flags_are_unchanged() {
    let mut terminal = emulator();
    terminal.advance(b"\x1b[=3u");
    let epoch = terminal.modes().keyboard_protocol_epoch;
    terminal.advance(b"ordinary output\x1b[?u\x1b[?4m");
    assert_eq!(terminal.modes().keyboard_protocol_epoch, epoch);
    terminal.advance(b"\x1b[=1u\x1b[=3u");
    assert_eq!(terminal.modes().keyboard.kitty_flags(), 3);
    assert_ne!(terminal.modes().keyboard_protocol_epoch, epoch);
    for sequence in [
        b"\x1b[>3u\x1b[>3u\x1b[<u".as_slice(),
        b"\x1b[>4;0m",
        b"\x1b[?1049h\x1b[?1049l",
        b"\x1bc",
    ] {
        let epoch = terminal.modes().keyboard_protocol_epoch;
        terminal.advance(sequence);
        assert_ne!(terminal.modes().keyboard_protocol_epoch, epoch);
    }
}
