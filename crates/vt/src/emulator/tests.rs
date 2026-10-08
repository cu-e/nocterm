use super::*;

#[cfg(test)]
mod screen {
    use super::*;

    fn emulator(cols: u16, rows: u16) -> Emulator {
        Emulator::new(TermSize::new(cols, rows, 8, 16), EmulatorOptions::default())
    }

    fn frame(emulator: &Emulator) -> Frame {
        let mut frame = Frame::default();
        emulator.snapshot(&mut frame);
        frame
    }

    #[test]
    fn unchanged_options_preserve_generation_and_changes_update_the_cursor() {
        let mut terminal = emulator(20, 2);
        terminal.advance(b"error");
        let generation = terminal.generation();
        terminal.set_options(EmulatorOptions::default());
        assert_eq!(terminal.generation(), generation);
        terminal.set_options(EmulatorOptions {
            cursor_blink: false,
            ..Default::default()
        });
        assert_eq!(terminal.generation(), generation + 1);
        assert!(!frame(&terminal).cursor.unwrap().blinking);
    }

    #[test]
    fn prints_text_and_moves_the_cursor() {
        let mut emulator = emulator(20, 4);
        emulator.advance(b"hello\r\nworld");

        let frame = frame(&emulator);

        assert_eq!(frame.row_text(0), "hello");
        assert_eq!(frame.row_text(1), "world");
        let cursor = frame.cursor.unwrap();
        assert_eq!((cursor.row, cursor.col), (1, 5));
    }

    #[test]
    fn maps_sgr_attributes_to_cells() {
        let mut emulator = emulator(20, 2);
        emulator.advance(b"\x1b[1;31;44mA\x1b[0m\x1b[38;2;1;2;3mB\x1b[38;5;196mC\x1b[7mD");

        let frame = frame(&emulator);
        let row = frame.row(0);

        assert_eq!(row[0].fg, Color::Ansi(1));
        assert_eq!(row[0].bg, Color::Ansi(4));
        assert!(row[0].style.bold);
        assert_eq!(row[1].fg, Color::Rgb(Rgb::new(1, 2, 3)));
        assert_eq!(row[1].bg, Color::Background);
        assert_eq!(row[2].fg, Color::Rgb(Rgb::new(0xff, 0, 0)));
        assert!(row[3].style.inverse);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let mut emulator = emulator(10, 2);
        emulator.advance("日x".as_bytes());

        let frame = frame(&emulator);
        let row = frame.row(0);

        assert!(row[0].wide && !row[0].spacer);
        assert!(row[1].spacer);
        assert_eq!(row[2].ch, 'x');
        assert_eq!(frame.row_text(0), "日x");
    }

    #[test]
    fn combining_characters_are_reported_beside_their_cell() {
        let mut emulator = emulator(10, 2);
        emulator.advance("e\u{301}".as_bytes());

        let frame = frame(&emulator);

        assert_eq!(frame.combining, [(0, "\u{301}".to_owned())]);
    }

    #[test]
    fn combining_flood_is_bounded_in_grid_snapshot_and_selection() {
        let mut emulator = emulator(2, 1);
        emulator.advance(b"a");
        let marks = "\u{301}".repeat(100_000);
        for chunk in marks.as_bytes().chunks(997) {
            emulator.advance(chunk);
        }
        assert_eq!(
            emulator.term.grid()[Point::new(Line(0), Column(0))]
                .zerowidth()
                .unwrap()
                .len(),
            64
        );
        emulator.advance(b"X");
        let frame = frame(&emulator);
        assert_eq!(frame.combining, [(0, "\u{301}".repeat(64))]);
        assert_eq!(frame.row_text(0), "aX");
        emulator.select_all();
        assert_eq!(
            emulator.selection_text().unwrap(),
            format!("a{}X", "\u{301}".repeat(64))
        );
    }

    #[test]
    fn ordinary_combining_clusters_preserve_wide_and_hidden_attributes() {
        let mut emulator = emulator(10, 1);
        emulator.advance("e\u{301}\u{302}日\u{301}\x1b[8mA\u{301}".as_bytes());
        let frame = frame(&emulator);
        assert_eq!(
            frame.combining,
            [
                (0, "\u{301}\u{302}".into()),
                (1, "\u{301}".into()),
                (3, "\u{301}".into())
            ]
        );
        assert!(frame.cells[1].wide);
        assert!(frame.cells[3].style.hidden);
    }

    #[test]
    fn oversized_osc_metadata_preserves_title_and_ends_hyperlinks() {
        let mut emulator = emulator(10, 1);
        assert_eq!(
            emulator.advance(b"\x1b]0;old\x07"),
            [Effect::Title(Some("old".into()))]
        );
        assert!(
            emulator
                .advance(format!("\x1b]0;{}\x07", "x".repeat(4097)).as_bytes())
                .is_empty()
        );
        emulator.advance(b"\x1b[22;0t\x1b]0;new\x07");
        assert_eq!(
            emulator.advance(b"\x1b[23;0t"),
            [Effect::Title(Some("old".into()))]
        );
        emulator.advance(b"\x1b]8;id=normal;https://example.org\x07A");
        emulator.advance(format!("\x1b]8;;{}\x07B", "x".repeat(4097)).as_bytes());
        emulator.advance(b"\x1b]8;;https://example.org\x07C");
        emulator
            .advance(format!("\x1b]8;id={};https://example.org\x07D", "x".repeat(1025)).as_bytes());
        for column in [0, 2] {
            assert!(
                emulator.term.grid()[Point::new(Line(0), Column(column))]
                    .hyperlink()
                    .is_some()
            );
        }
        for column in [1, 3] {
            assert!(
                emulator.term.grid()[Point::new(Line(0), Column(column))]
                    .hyperlink()
                    .is_none()
            );
        }
        assert_eq!(frame(&emulator).row_text(0), "ABCD");
    }

    #[test]
    fn reports_titles_bells_and_query_replies() {
        let mut emulator = emulator(20, 4);

        let effects = emulator.advance(b"\x1b]0;build\x07\x07\x1b[6n");

        assert_eq!(
            effects,
            [
                Effect::Title(Some("build".to_owned())),
                Effect::Bell,
                Effect::Reply(b"\x1b[1;1R".to_vec()),
            ]
        );
    }

    #[test]
    fn answers_colour_queries_from_the_host_palette() {
        let mut emulator = emulator(20, 4);
        emulator.set_palette(Palette {
            background: Rgb::new(0x10, 0x20, 0x30),
            ..Palette::default()
        });

        let effects = emulator.advance(b"\x1b]11;?\x07");

        assert_eq!(
            effects,
            [Effect::Reply(b"\x1b]11;rgb:1010/2020/3030\x07".to_vec())]
        );
    }

    #[test]
    fn tracks_the_modes_that_change_input_encoding() {
        let mut emulator = emulator(20, 4);
        assert_eq!(
            emulator.modes(),
            Modes {
                alternate_scroll: true,
                ..Modes::default()
            }
        );

        emulator.advance(b"\x1b[?1h\x1b[?2004h\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?1004h");

        let modes = emulator.modes();
        assert!(modes.app_cursor && modes.bracketed_paste && modes.alt_screen);
        assert!(modes.mouse_clicks && modes.mouse_sgr && modes.focus_events);
        assert!(modes.reports_mouse());
    }

    #[test]
    fn scrolls_through_history() {
        let mut emulator = emulator(10, 3);
        for line in 0..10 {
            emulator.advance(format!("line{line}\r\n").as_bytes());
        }
        assert_eq!(frame(&emulator).row_text(0), "line8");

        emulator.scroll(Scroll::Lines(2));
        let scrolled = frame(&emulator);
        assert_eq!(scrolled.display_offset, 2);
        assert_eq!(scrolled.row_text(0), "line6");
        assert_eq!(scrolled.cursor, None, "the cursor scrolled out of view");

        emulator.scroll(Scroll::Bottom);
        assert_eq!(emulator.display_offset(), 0);
        assert_eq!(frame(&emulator).history_size, 8);
    }

    #[test]
    fn selects_cells_words_and_lines() {
        let mut emulator = emulator(20, 3);
        emulator.advance(b"alpha beta\r\ngamma");

        emulator.start_selection(
            SelectionKind::Cells,
            CellPoint { row: 0, col: 1 },
            Side::Left,
        );
        emulator.update_selection(CellPoint { row: 0, col: 3 }, Side::Right);
        assert_eq!(emulator.selection_text().as_deref(), Some("lph"));
        let selected: Vec<bool> = frame(&emulator).row(0)[..5]
            .iter()
            .map(|c| c.selected)
            .collect();
        assert_eq!(selected, [false, true, true, true, false]);

        emulator.start_selection(
            SelectionKind::Words,
            CellPoint { row: 0, col: 7 },
            Side::Left,
        );
        assert_eq!(emulator.selection_text().as_deref(), Some("beta"));

        emulator.start_selection(
            SelectionKind::Lines,
            CellPoint { row: 1, col: 2 },
            Side::Left,
        );
        assert_eq!(emulator.selection_text().as_deref(), Some("gamma\n"));

        emulator.clear_selection();
        assert_eq!(emulator.selection_text(), None);
    }

    #[test]
    fn resizing_reflows_and_clamps_to_a_minimum() {
        let mut emulator = emulator(10, 3);
        emulator.advance(b"0123456789abc");

        emulator.resize(TermSize::new(0, 0, 8, 16));
        assert_eq!((emulator.size().cols, emulator.size().rows), (2, 1));

        emulator.resize(TermSize::new(20, 3, 8, 16));
        assert_eq!(frame(&emulator).row_text(0), "0123456789abc");
    }

    #[test]
    fn default_cursor_follows_the_options() {
        let mut emulator = Emulator::new(
            TermSize::default(),
            EmulatorOptions {
                cursor_shape: CursorShape::Bar,
                cursor_blink: false,
                ..EmulatorOptions::default()
            },
        );
        let cursor = frame(&emulator).cursor.unwrap();
        assert_eq!((cursor.shape, cursor.blinking), (CursorShape::Bar, false));

        emulator.advance(b"\x1b[?25l");
        assert_eq!(frame(&emulator).cursor, None);
    }

    #[test]
    fn indexed_colours_follow_the_xterm_table() {
        assert_eq!(indexed_color(16), Rgb::new(0, 0, 0));
        assert_eq!(indexed_color(21), Rgb::new(0, 0, 255));
        assert_eq!(indexed_color(231), Rgb::new(255, 255, 255));
        assert_eq!(indexed_color(232), Rgb::new(8, 8, 8));
        assert_eq!(indexed_color(255), Rgb::new(238, 238, 238));
    }
}

#[cfg(test)]
mod line_metadata_tests {
    use super::*;
    fn emulator(cols: u16, rows: u16) -> Emulator {
        Emulator::new(TermSize::new(cols, rows, 0, 0), EmulatorOptions::default())
    }
    fn frame(emulator: &Emulator) -> Frame {
        let mut frame = Frame::default();
        emulator.snapshot(&mut frame);
        frame
    }

    #[test]
    fn wrapped_rows_keep_first_observation_across_fragmented_output() {
        let mut terminal = emulator(4, 4);
        terminal.advance_at(b"ab", 1000);
        terminal.advance_at(b"cdefgh\r\nx", 9000);
        let frame = frame(&terminal);
        assert_eq!(
            frame.lines[0],
            Some(LineMetadata {
                number: 1,
                timestamp_ms: 1000,
                continuation: false
            })
        );
        assert_eq!(
            frame.lines[1],
            Some(LineMetadata {
                number: 1,
                timestamp_ms: 1000,
                continuation: true
            })
        );
        assert_eq!(
            frame.lines[2],
            Some(LineMetadata {
                number: 2,
                timestamp_ms: 9000,
                continuation: false
            })
        );
    }
    #[test]
    fn reflow_and_blank_output_keep_identity() {
        let mut terminal = emulator(8, 5);
        terminal.advance_at(b"abcdefghijk\r\n\r\nx", 1000);
        let before = frame(&terminal);
        assert_eq!(before.lines[2].unwrap().number, 2);
        assert_eq!(before.lines[3].unwrap().number, 3);
        terminal.resize(TermSize::new(4, 5, 0, 0));
        let narrower = frame(&terminal);
        assert!(
            narrower
                .lines
                .iter()
                .flatten()
                .filter(|line| line.number == 1)
                .all(|line| line.timestamp_ms == 1000)
        );
        assert!(
            narrower.lines.iter().flatten().any(|line| line.number == 2),
            "explicit blank LF survives reflow"
        );
        assert!(narrower.lines.iter().flatten().any(|line| line.number == 3));
        terminal.resize(TermSize::new(16, 5, 0, 0));
        let wider = frame(&terminal);
        assert!(wider.lines.iter().flatten().any(|line| line.number == 1));
        assert!(wider.lines.iter().flatten().any(|line| line.number == 2));
        assert!(wider.lines.iter().flatten().any(|line| line.number == 3));
    }
    #[test]
    fn redraw_and_character_edits_preserve_line_but_screen_clear_is_fresh() {
        let mut terminal = emulator(20, 4);
        terminal.advance_at(b"old", 1000);
        for edits in [
            b"\r\x1b[2Knew".as_slice(),
            b"\r\x1b[3Xnew",
            b"\r\x1b[3Pnew",
            b"\r\x1b[2@new",
        ] {
            terminal.advance_at(edits, 2000);
            assert_eq!(frame(&terminal).lines[0].unwrap().number, 1);
            assert_eq!(frame(&terminal).lines[0].unwrap().timestamp_ms, 1000);
        }
        terminal.advance_at(b"\x1b[2J\x1b[Hfresh", 3000);
        assert_eq!(frame(&terminal).lines[0].unwrap().number, 2);
        assert_eq!(frame(&terminal).lines[0].unwrap().timestamp_ms, 3000);
    }
    #[test]
    fn trimming_history_never_renumbers_remaining_lines() {
        let options = EmulatorOptions {
            scrollback_lines: 2,
            ..EmulatorOptions::default()
        };
        let mut terminal = Emulator::new(TermSize::new(10, 2, 0, 0), options);
        for number in 1..=12 {
            terminal.advance_at(format!("{number}\r\n").as_bytes(), number * 1000);
        }
        assert_eq!(terminal.output_line_number(), 12);
        terminal.scroll(Scroll::Top);
        let top = frame(&terminal);
        assert!(top.lines.iter().flatten().all(|line| line.number >= 9));
        assert_eq!(top.history_size, 2);
    }
    #[test]
    fn alternate_screen_hides_metadata_and_does_not_consume_normal_numbers() {
        let mut terminal = emulator(8, 3);
        terminal.advance_at(b"normal", 1000);
        terminal.advance_at(b"\x1b[?1049hother\r\nrows", 2000);
        let alt = frame(&terminal);
        assert!(alt.alt_screen);
        assert!(alt.lines.iter().all(Option::is_none));
        assert_eq!(terminal.output_line_number(), 1);
        terminal.advance_at(b"\x1b[?1049l\r\nnext", 3000);
        let normal = frame(&terminal);
        assert!(!normal.alt_screen);
        assert_eq!(normal.lines[0].unwrap().number, 1);
        assert_eq!(normal.lines[1].unwrap().number, 2);
        terminal.start_selection(
            SelectionKind::Lines,
            CellPoint { row: 1, col: 0 },
            Side::Left,
        );
        assert!(!terminal.selection_text().unwrap().contains("3000"));
    }
}

#[cfg(test)]
mod lineage_storage_tests {
    use super::*;
    #[test]
    fn ordinary_characters_share_metadata_and_attribute_reset_keeps_it() {
        let mut terminal = Emulator::new(TermSize::new(10, 2, 0, 0), EmulatorOptions::default());
        terminal.advance_at(b"abc", 1000);
        let row = &terminal.term.grid()[alacritty_terminal::index::Line(0)];
        assert!(Arc::ptr_eq(
            row[Column(0)].extra.as_ref().unwrap(),
            row[Column(1)].extra.as_ref().unwrap()
        ));
        let mut cell = row[Column(0)].clone();
        let lineage = cell.logical_line();
        cell.set_underline_color(Some(AnsiColor::Named(NamedColor::Red)));
        cell.set_underline_color(None);
        cell.set_hyperlink(None);
        assert_eq!(cell.logical_line(), lineage);
        assert!(std::mem::size_of::<alacritty_terminal::term::cell::Cell>() <= 24);
    }
}

#[cfg(test)]
mod osc_limit_tests {
    use super::*;
    fn emulator() -> Emulator {
        Emulator::new(TermSize::new(80, 3, 0, 0), EmulatorOptions::default())
    }
    #[test]
    fn oversize_title_and_clipboard_never_dispatch_and_each_terminator_recovers() {
        for prefix in [b"\x1b]0;".as_slice(), b"\x1b]52;c;".as_slice()] {
            for terminator in [0x07, 0x18, 0x1a, 0x1b] {
                let mut terminal = emulator();
                assert!(terminal.advance(prefix).is_empty());
                for _ in 0..256 {
                    assert!(terminal.advance(&[b'A'; 8192]).is_empty());
                }
                let mut recovery = vec![terminator];
                if terminator == 0x1b {
                    recovery.push(b'\\');
                }
                recovery.extend(b"restored\x1b]0;new title\x07");
                let effects = terminal.advance(&recovery);
                assert!(
                    matches!(effects.as_slice(), [Effect::Title(title)] if title.as_deref() == Some("new title"))
                );
                let mut frame = Frame::default();
                terminal.snapshot(&mut frame);
                assert_eq!(frame.row_text(0), "restored");
            }
        }
    }
    #[test]
    fn valid_title_at_the_metadata_limit_is_not_truncated() {
        let mut terminal = emulator();
        terminal.advance(b"\x1b]0;");
        let title = vec![b'x'; 4096];
        assert!(terminal.advance(&title).is_empty());
        let effects = terminal.advance(b"\x07");
        assert!(
            matches!(effects.as_slice(), [Effect::Title(Some(value))] if value.len() == title.len())
        );
    }
    #[test]
    fn each_escape_follower_preserves_upstream_semantics_below_the_limit() {
        for byte in 0..=255 {
            let mut guarded = emulator();
            let mut native = emulator();
            let bytes = [
                b"before\x1b".as_slice(),
                &[byte],
                b"]0;x\x07after\x1b]52;c;b2s=\x1b\\",
            ]
            .concat();
            let mut guarded_effects = Vec::new();
            let mut native_effects = Vec::new();
            for byte in bytes {
                guarded_effects.extend(guarded.advance_at(&[byte], 0));
                native.term.set_output_timestamp_ms(0);
                native.parser.advance(&mut native.term, &[byte]);
                native_effects.extend(native.take_effects(0));
            }
            assert_eq!(guarded_effects, native_effects, "escape follower {byte:#x}");
            let (mut left, mut right) = (Frame::default(), Frame::default());
            guarded.snapshot(&mut left);
            native.snapshot(&mut right);
            assert_eq!(left, right, "escape follower {byte:#x}");
        }
    }
    #[test]
    fn ordinary_fragmented_osc_and_utf8_preserve_native_effects_and_synchronized_output() {
        let bytes = "\x1b[?2026hλ\x1b]0;title λ\x1b\\\x1b]52;c;b2s=\x07\x1b[?2026l".as_bytes();
        let mut terminal = emulator();
        let effects: Vec<_> = bytes
            .iter()
            .flat_map(|byte| terminal.advance(&[*byte]))
            .collect();
        assert!(effects.iter().any(
            |effect| matches!(effect, Effect::Title(title) if title.as_deref() == Some("title λ"))
        ));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::CopyToClipboard(text) if text == "ok"))
        );
        let mut frame = Frame::default();
        terminal.snapshot(&mut frame);
        assert_eq!(frame.row_text(0), "λ");
        assert!(terminal.sync_deadline().is_none());
    }
}
