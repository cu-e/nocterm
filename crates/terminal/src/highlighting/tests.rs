use super::*;
use nocterm_vt::{Emulator, EmulatorOptions, Scroll, SelectionKind, Side};

fn snapshot(text: &str, cols: u16, rows: u16) -> (Emulator, Frame, Highlights) {
    let mut emulator = Emulator::new(TermSize::new(cols, rows, 8, 16), EmulatorOptions::default());
    emulator.advance(text.as_bytes());
    let mut frame = Frame::default();
    emulator.snapshot(&mut frame);
    let mut highlights = Highlights::default();
    highlights.update(&frame, emulator.generation(), true);
    (emulator, frame, highlights)
}

fn role(text: &str, needle: &str) -> Option<Role> {
    let (_, frame, highlights) = snapshot(text, 250, 2);
    let index = text[..text.find(needle).unwrap()].chars().count();
    highlights.role_at(index, &frame.cells[index])
}

#[test]
fn screenshot_motd_and_syslog_fields_are_meaningful() {
    let motd = "Linux cv758185.novalocal 6.12.43+deb13-amd64 #1 SMP PREEMPT_DYNAMIC Debian 6.12.43-1 (2025-08-27) x86_64";
    assert_eq!(role(motd, "6.12.43+"), Some(Role::Version));
    assert_eq!(role(motd, "2025-08-27"), Some(Role::Timestamp));
    assert_eq!(role(motd, "cv758185"), None);
    assert_eq!(
        role(
            "Debian GNU/Linux comes with ABSOLUTELY NO WARRANTY",
            "NO WARRANTY"
        ),
        Some(Role::Error)
    );
    let log = "Oct 07 15:59:33 cv758185.novalocal sshd-session[460573]: Failed password for root from 136.232.11.10 port 51263 ssh2";
    for (token, expected) in [
        ("Oct 07", Role::Timestamp),
        ("15:59:33", Role::Timestamp),
        ("460573", Role::Identifier),
        ("Failed password", Role::Error),
        ("136.232.11.10", Role::Address),
        ("51263", Role::Identifier),
    ] {
        assert_eq!(role(log, token), Some(expected), "{token}");
    }
    for (text, expected) in [
        ("Accepted password", Role::Success),
        (
            "pam_unix(sshd:session): session opened for user root(uid=0)",
            Role::Info,
        ),
        (
            "Couldn't stat /var/log/lastlog: No such file or directory",
            Role::Error,
        ),
        ("Connection closed by 82.194.49.43 port 48406", Role::Info),
        ("WARNING: timed out", Role::Warning),
    ] {
        let needle = if text.starts_with("pam_") {
            "session opened"
        } else {
            text
        };
        assert_eq!(role(text, needle), Some(expected));
    }
    assert_eq!(role("root(uid=0) by (uid=0)", "0"), Some(Role::Identifier));
}

#[test]
fn validates_addresses_and_avoids_subword_and_number_noise() {
    for ip in ["127.0.0.1", "2001:db8::1", "::1", "::ffff:192.0.2.1"] {
        assert_eq!(role(&format!("from {ip} to host"), ip), Some(Role::Address));
    }
    for text in [
        "999.12.0.1",
        "1.2.3.999",
        "foo127.0.0.1bar",
        "errors",
        "failedness",
        "123456",
        "25:99:33",
    ] {
        assert_eq!(role(text, text), None, "{text}");
    }
}

#[test]
fn wrapped_tokens_on_both_screens_and_unicode_map_to_cells() {
    for prefix in ["", "\x1b[?1049h"] {
        let (_, frame, highlights) = snapshot(&format!("{prefix}Failed password"), 8, 3);
        assert!(frame.wraps[0]);
        for index in 0..15 {
            assert_eq!(
                highlights.role_at(index, &frame.cells[index]),
                Some(Role::Error)
            );
        }
    }
    let (_, frame, highlights) = snapshot("日e\u{301} from 192.0.2.1", 40, 2);
    assert_eq!(highlights.role_at(9, &frame.cells[9]), Some(Role::Address));
    assert!(frame.cells[1].spacer);
    assert_eq!(frame.combining, [(2, "\u{301}".into())]);
    assert_eq!(frame.row_text(0), "日e from 192.0.2.1");
}

#[test]
fn preserves_program_styles_selection_search_and_clipboard() {
    for sgr in [
        "31",
        "38;2;220;10;10",
        "44",
        "1",
        "2",
        "3",
        "4",
        "7",
        "8",
        "9",
    ] {
        let (_, frame, highlights) = snapshot(&format!("\x1b[{sgr}mFailed password"), 40, 2);
        assert_eq!(highlights.role_at(0, &frame.cells[0]), None, "{sgr}");
    }
    let (mut emulator, mut frame, mut highlights) = snapshot("Failed password", 40, 2);
    emulator.start_selection(
        SelectionKind::Cells,
        nocterm_vt::CellPoint { row: 0, col: 0 },
        Side::Left,
    );
    emulator.update_selection(nocterm_vt::CellPoint { row: 0, col: 14 }, Side::Right);
    emulator.snapshot(&mut frame);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(
        emulator.selection_text().as_deref(),
        Some("Failed password")
    );
    assert_eq!(highlights.role_at(0, &frame.cells[0]), None);
    frame.cells[0].selected = false;
    frame.cells[0].search_hit = true;
    assert_eq!(highlights.role_at(0, &frame.cells[0]), None);
    assert_eq!(
        highlights.scans, 1,
        "selection never invalidates content cache"
    );
}

#[test]
fn cache_handles_output_scroll_resize_alt_screen_and_disable_without_idle_work() {
    let (mut emulator, mut frame, mut highlights) = snapshot("error\r\nwarning\r\nsuccess", 20, 2);
    for _ in 0..20 {
        frame.cursor = None;
        highlights.update(&frame, emulator.generation(), true);
    }
    assert_eq!(highlights.scans, 1);
    emulator.scroll(Scroll::Top);
    emulator.snapshot(&mut frame);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(highlights.scans, 2);
    emulator.resize(TermSize::new(10, 2, 8, 16));
    emulator.snapshot(&mut frame);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(highlights.scans, 3);
    emulator.advance(b"\x1b[?1049herror");
    emulator.snapshot(&mut frame);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(highlights.scans, 4);
    highlights.update(&frame, emulator.generation(), false);
    assert_eq!(highlights.spans.capacity(), 0);
    assert_eq!(highlights.scans, 4);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(highlights.scans, 5);
}

#[test]
fn bounds_group_scratch_viewport_work_and_retained_spans() {
    let (_, frame, highlights) = snapshot(&format!("{} error", "a".repeat(GROUP_BYTES)), 80, 60);
    assert!(frame.wraps[0]);
    assert!(
        highlights.spans.is_empty(),
        "oversized logical groups are skipped"
    );
    let text = "error 192.0.2.1 ".repeat(6000);
    let (_, _, highlights) = snapshot(&text, 80, 1200);
    assert!(highlights.spans.len() <= MAX_SPANS);
    assert!(std::mem::size_of::<Span>() <= 16);
    assert!(highlights.spans.capacity() <= MAX_SPANS);
}

#[test]
fn hidden_cells_break_phrases_and_iso_datetime_is_one_timestamp() {
    let (_, frame, highlights) = snapshot("Failed\x1b[8m \x1b[0mpassword", 40, 2);
    assert_eq!(highlights.role_at(7, &frame.cells[7]), None);
    assert_eq!(role("2026-10-07T15:59:33Z", "2026"), Some(Role::Timestamp));
    assert_eq!(
        role("2026-10-07T15:59:33+05:00", "2026"),
        Some(Role::Timestamp)
    );
}

#[test]
fn palette_and_unchanged_options_keep_cached_semantic_roles() {
    let (mut emulator, mut frame, mut highlights) = snapshot("Failed password", 40, 2);
    emulator.set_palette(nocterm_vt::Palette::default());
    emulator.set_options(EmulatorOptions::default());
    emulator.snapshot(&mut frame);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(highlights.scans, 1);
    assert_eq!(highlights.role_at(0, &frame.cells[0]), Some(Role::Error));
    emulator.set_options(EmulatorOptions {
        cursor_blink: false,
        ..Default::default()
    });
    emulator.snapshot(&mut frame);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(highlights.scans, 2);
}

#[test]
fn clipped_wrapped_prefix_does_not_fabricate_error_boundary() {
    // The visible first row continues a word whose prefix is in scrollback.
    let (_, frame, highlights) = snapshot("supererror\r\nnext", 5, 2);
    assert_eq!(frame.row_text(0), "error");
    assert_eq!(highlights.role_at(0, &frame.cells[0]), None);
}

#[test]
fn clipped_wrapped_suffix_does_not_fabricate_error_boundary() {
    // The visible last row is only the prefix of a word below the viewport.
    let (mut emulator, mut frame, mut highlights) = snapshot("zero\r\nerrorness", 5, 2);
    emulator.scroll(Scroll::Top);
    emulator.snapshot(&mut frame);
    assert_eq!(frame.row_text(1), "error");
    assert!(frame.wraps[1]);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(highlights.role_at(5, &frame.cells[5]), None);
}

#[test]
fn clipped_group_keeps_tokens_with_known_internal_boundaries() {
    let (_, frame, highlights) = snapshot("aaaaaaaaaaerror warn\r\nnext", 10, 2);
    assert_eq!(frame.row_text(0), "error warn");
    assert_eq!(highlights.role_at(0, &frame.cells[0]), None);
    assert_eq!(highlights.role_at(6, &frame.cells[6]), Some(Role::Warning));

    let (mut emulator, mut frame, mut highlights) = snapshot("zero\r\nwarn errorness", 10, 2);
    emulator.scroll(Scroll::Top);
    emulator.snapshot(&mut frame);
    assert_eq!(frame.row_text(1), "warn error");
    assert!(frame.wraps[1]);
    highlights.update(&frame, emulator.generation(), true);
    assert_eq!(
        highlights.role_at(10, &frame.cells[10]),
        Some(Role::Warning)
    );
    assert_eq!(highlights.role_at(15, &frame.cells[15]), None);
}

fn repeated_rows(text: &str, cols: u16, rows: u16) -> Frame {
    let mut frame = Frame {
        size: TermSize::new(cols, rows, 8, 16),
        cells: vec![Cell::default(); usize::from(cols) * usize::from(rows)],
        wraps: vec![false; usize::from(rows)],
        ..Default::default()
    };
    for row in frame.cells.chunks_mut(usize::from(cols)) {
        for (cell, ch) in row.iter_mut().zip(text.chars()) {
            cell.ch = ch;
        }
    }
    frame
}

#[test]
fn budgets_allow_exact_limit_and_stop_before_later_valid_tokens() {
    let frame = repeated_rows("error", 256, 258);
    let mut highlights = Highlights::default();
    highlights.update(&frame, 1, true);
    let last = 255 * 256;
    assert_eq!(
        highlights.role_at(last, &frame.cells[last]),
        Some(Role::Error)
    );
    let beyond = 256 * 256;
    assert_eq!(highlights.role_at(beyond, &frame.cells[beyond]), None);

    let mut frame = repeated_rows(&format!("error {}", "e".repeat(250)), 256, 80);
    for row in 0..80 {
        for col in 6..256 {
            frame
                .combining
                .push((row * 256 + col, "\u{301}\u{301}".into()));
        }
    }
    highlights.update(&frame, 2, true);
    // Each complete row contributes 6 ASCII bytes + 250 * 5 combining bytes.
    let complete = (FRAME_BYTES / 1256 - 1) * 256;
    let clipped = (FRAME_BYTES / 1256) * 256;
    assert_eq!(
        highlights.role_at(complete, &frame.cells[complete]),
        Some(Role::Error)
    );
    assert_eq!(highlights.role_at(clipped, &frame.cells[clipped]), None);
}

#[test]
fn span_limit_is_reached_without_losing_earlier_roles() {
    let frame = repeated_rows("error warning success", 24, 1500);
    let mut highlights = Highlights::default();
    highlights.update(&frame, 1, true);
    assert_eq!(highlights.spans.len(), MAX_SPANS);
    assert_eq!(highlights.spans.capacity(), MAX_SPANS);
    assert_eq!(highlights.role_at(0, &frame.cells[0]), Some(Role::Error));
    let beyond = 1400 * 24;
    assert_eq!(highlights.role_at(beyond, &frame.cells[beyond]), None);
    highlights.update(&frame, 1, false);
    assert!(highlights.spans.is_empty());
    assert_eq!(highlights.spans.capacity(), 0);
}

#[test]
fn oversized_logical_groups_discard_a_valid_token_at_the_start() {
    let (_, frame, highlights) = snapshot(&format!("error {}", "x".repeat(GROUP_BYTES)), 80, 60);
    assert!(frame.wraps[0]);
    assert_eq!(highlights.role_at(0, &frame.cells[0]), None);
}

#[test]
fn token_limit_stops_at_512_complete_matches() {
    let (_, frame, highlights) = snapshot(&"error ".repeat(513), 160, 22);
    assert_eq!(highlights.spans.len(), 512);
    let last = 511 * 6;
    assert_eq!(
        highlights.role_at(last, &frame.cells[last]),
        Some(Role::Error)
    );
    let beyond = 512 * 6;
    assert_eq!(highlights.role_at(beyond, &frame.cells[beyond]), None);
}

#[test]
fn bracketed_and_cidr_addresses_have_independent_valid_boundaries() {
    for (text, needle) in [
        ("[2001:db8::1]:22", "2001:db8::1"),
        ("192.0.2.1/24", "192.0.2.1"),
        ("[192.0.2.1]", "192.0.2.1"),
        ("::ffff:192.0.2.1/128", "::ffff:192.0.2.1"),
    ] {
        assert_eq!(role(text, needle), Some(Role::Address), "{text}");
    }
}

#[test]
fn combining_word_characters_do_not_create_address_boundaries() {
    for text in ["e\u{301}192.0.2.1", "192.0.2.1\u{301}e"] {
        let (_, frame, highlights) = snapshot(text, 40, 2);
        let index = usize::from(text.starts_with('e'));
        assert_eq!(
            highlights.role_at(index, &frame.cells[index]),
            None,
            "{text}"
        );
    }
}

#[test]
fn clipped_wraps_preserve_complete_tokens_inside_the_visible_group() {
    let (_, frame, highlights) = snapshot("supererror error", 5, 3);
    assert!(frame.starts_with_continuation);
    assert_eq!(highlights.role_at(0, &frame.cells[0]), None);
    assert_eq!(highlights.role_at(6, &frame.cells[6]), Some(Role::Error));

    let mut frame = repeated_rows("", 6, 2);
    for (cell, ch) in frame.cells.iter_mut().zip("error  error".chars()) {
        cell.ch = ch;
    }
    frame.wraps = vec![true, true];
    let mut highlights = Highlights::default();
    highlights.update(&frame, 0, true);
    assert_eq!(highlights.role_at(0, &frame.cells[0]), Some(Role::Error));
    assert_eq!(highlights.role_at(7, &frame.cells[7]), None);
}

#[test]
fn unicode_marks_join_controls_and_connectors_keep_word_boundaries_consistent() {
    for word in [
        "x\u{20dd}192.0.2.1",
        "192.0.2.1\u{93e}x",
        "x\u{200c}192.0.2.1",
        "192.0.2.1\u{200d}x",
        "x\u{203f}192.0.2.1",
    ] {
        let (_, frame, highlights) = snapshot(word, 40, 2);
        assert!(
            frame
                .cells
                .iter()
                .enumerate()
                .all(|(index, cell)| { highlights.role_at(index, cell).is_none() }),
            "{word}"
        );
    }
    let (_, frame, highlights) = snapshot("日 192.0.2.1", 40, 2);
    assert_eq!(highlights.role_at(3, &frame.cells[3]), Some(Role::Address));
}
