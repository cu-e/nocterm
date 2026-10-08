use super::*;
use crate::{EmulatorOptions, TermSize};
fn finish(
    e: &Emulator,
    query: &str,
    direction: SearchDirection,
    anchor: Option<SearchPoint>,
) -> SearchResult {
    let mut scan = e.search(query, direction, anchor).unwrap();
    loop {
        if let SearchProgress::Complete(result) = scan.step(e, 7) {
            return result;
        }
    }
}
fn finish_options(
    e: &Emulator,
    query: &str,
    options: SearchOptions,
    direction: SearchDirection,
    anchor: Option<SearchPoint>,
) -> Result<SearchResult, String> {
    let mut scan = e.search_with_options(query, options, direction, anchor)?;
    loop {
        match scan.step(e, 4096) {
            SearchProgress::Complete(result) => return Ok(result),
            SearchProgress::Failed(error) => return Err(error),
            SearchProgress::Work(work) => {
                scan.accept_work(e, work.run());
            }
            SearchProgress::Invalidated => panic!("unchanged grid was invalidated"),
            SearchProgress::Searching => {}
        }
    }
}

#[test]
fn unicode_case_folding_keeps_overlap_and_cell_positions() {
    let mut e = Emulator::new(TermSize::new(80, 3, 0, 0), EmulatorOptions::default());
    e.advance("Кот кОт КОТ Σσς KkK aAa".as_bytes());
    let options = SearchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    assert_eq!(
        finish_options(&e, "кот", options, SearchDirection::Next, None)
            .unwrap()
            .count,
        3
    );
    assert_eq!(
        finish_options(&e, "Σ", options, SearchDirection::Next, None)
            .unwrap()
            .count,
        3
    );
    assert_eq!(
        finish_options(&e, "k", options, SearchDirection::Next, None)
            .unwrap()
            .count,
        3
    );
    assert_eq!(
        finish_options(&e, "aa", options, SearchDirection::Next, None)
            .unwrap()
            .count,
        2
    );
    assert_eq!(finish(&e, "КОТ", SearchDirection::Next, None).count, 1);
}

#[test]
fn whole_word_treats_marks_digits_and_connectors_as_unicode_word_characters() {
    let mut e = Emulator::new(TermSize::new(80, 3, 0, 0), EmulatorOptions::default());
    e.advance("cat cat_ cat2 cat\u{301} caté cat-cat CAT".as_bytes());
    for regex in [false, true] {
        let options = SearchOptions {
            regex,
            case_sensitive: false,
            whole_word: true,
        };
        let result = finish_options(&e, "cat", options, SearchDirection::Next, None).unwrap();
        assert_eq!(result.count, 4, "regex={regex}");
        assert_eq!(result.active.unwrap().start.column, 0);
    }
}

#[test]
fn regex_anchors_join_soft_wraps_and_map_utf8_combining_bytes_to_cells() {
    let mut e = Emulator::new(TermSize::new(6, 3, 0, 0), EmulatorOptions::default());
    e.advance("ab界e\u{301}fg\r\nlast".as_bytes());
    let options = SearchOptions {
        regex: true,
        ..Default::default()
    };
    let result =
        finish_options(&e, "^ab界e\u{301}fg$", options, SearchDirection::Next, None).unwrap();
    assert_eq!(result.count, 1);
    let found = result.active.unwrap();
    assert_eq!(found.start.column, 0);
    assert_eq!(found.end.column, 0);
    assert_ne!(found.start.line, found.end.line);
    assert_eq!(
        finish_options(&e, "fg\\nlast", options, SearchDirection::Next, None)
            .unwrap()
            .count,
        0
    );
    assert_eq!(
        finish_options(&e, "界e\u{301}", options, SearchDirection::Next, None)
            .unwrap()
            .active
            .unwrap()
            .end
            .column,
        4
    );
}

#[test]
fn regex_navigation_wraps_without_retaining_a_match_list_or_selecting_empty_matches() {
    let mut e = Emulator::new(TermSize::new(80, 3, 0, 0), EmulatorOptions::default());
    e.advance(b"id1 id22 id333");
    let options = SearchOptions {
        regex: true,
        ..Default::default()
    };
    let first = finish_options(&e, r"id\d+", options, SearchDirection::Next, None).unwrap();
    assert_eq!(first.count, 3);
    let last = finish_options(
        &e,
        r"id\d+",
        options,
        SearchDirection::Previous,
        first.active.map(|found| found.start),
    )
    .unwrap();
    assert_eq!(last.ordinal, 3);
    assert_eq!(
        finish_options(
            &e,
            r"id\d+",
            options,
            SearchDirection::Next,
            last.active.map(|found| found.start)
        )
        .unwrap()
        .ordinal,
        1
    );
    assert_eq!(
        finish_options(&e, "^|$", options, SearchDirection::Next, None)
            .unwrap()
            .count,
        0
    );
    assert!(
        e.search_with_options("[", options, SearchDirection::Next, None)
            .is_err()
    );
    assert!(
        e.search_with_options("a{1000000}", options, SearchDirection::Next, None)
            .is_err()
    );
}

#[test]
fn regex_limit_is_explicit_and_greedy_long_line_is_one_bounded_match() {
    let mut e = Emulator::new(
        TermSize::new(100, 3, 0, 0),
        EmulatorOptions {
            scrollback_lines: 2000,
            ..Default::default()
        },
    );
    e.advance("a".repeat(MAX_REGEX_LINE_BYTES).as_bytes());
    let options = SearchOptions {
        regex: true,
        ..Default::default()
    };
    let result = finish_options(&e, "a*", options, SearchDirection::Next, None).unwrap();
    assert_eq!(result.count, 1);
    e.advance(b"a");
    assert!(
        finish_options(&e, "a*", options, SearchDirection::Next, None)
            .unwrap_err()
            .contains("65536 bytes")
    );
    assert_eq!(
        finish(&e, "aa", SearchDirection::Next, None).count,
        MAX_REGEX_LINE_BYTES as u64
    );
}

#[test]
fn regex_batches_many_short_rows_and_counts_every_match() {
    let mut e = Emulator::new(
        TermSize::new(12, 3, 0, 0),
        EmulatorOptions {
            scrollback_lines: 6000,
            ..Default::default()
        },
    );
    e.advance("id1\r\n".repeat(5000).as_bytes());
    let mut scan = e
        .search_with_options(
            r"id\d+",
            SearchOptions {
                regex: true,
                ..Default::default()
            },
            SearchDirection::Next,
            None,
        )
        .unwrap();
    let mut jobs = 0;
    let result = loop {
        match scan.step(&e, 4096) {
            SearchProgress::Work(work) => {
                jobs += 1;
                assert!(work.lines.len() > 1);
                assert!(
                    work.lines.iter().map(|line| line.text.len()).sum::<usize>()
                        <= MAX_REGEX_BATCH_BYTES
                );
                scan.accept_work(&e, work.run());
            }
            SearchProgress::Complete(result) => break result,
            SearchProgress::Searching => {}
            progress => panic!("unexpected {progress:?}"),
        }
    };
    assert_eq!(result.count, 5000);
    assert!(jobs < 100, "short rows were scheduled individually: {jobs}");
}

#[test]
fn abandoned_or_invalidated_regex_work_cannot_publish_results() {
    let mut e = Emulator::new(TermSize::new(80, 3, 0, 0), EmulatorOptions::default());
    e.advance(b"target");
    let options = SearchOptions {
        regex: true,
        ..Default::default()
    };
    let mut scan = e
        .search_with_options("target", options, SearchDirection::Next, None)
        .unwrap();
    let SearchProgress::Work(work) = scan.step(&e, 4096) else {
        panic!("expected work")
    };
    drop(scan);
    assert_eq!(work.run().unwrap_err(), "Search was cancelled.");
    let mut scan = e
        .search_with_options("target", options, SearchDirection::Next, None)
        .unwrap();
    let SearchProgress::Work(work) = scan.step(&e, 4096) else {
        panic!("expected work")
    };
    let result = work.run();
    e.advance(b"changed");
    assert_eq!(scan.accept_work(&e, result), SearchProgress::Invalidated);
}

#[test]
fn unicode_wide_combining_soft_wrap_and_hard_newline() {
    let mut e = Emulator::new(TermSize::new(6, 3, 0, 0), EmulatorOptions::default());
    e.advance("ab界e\u{301}fg\r\nlast".as_bytes());
    let result = finish(&e, "界e\u{301}fg", SearchDirection::Next, None);
    assert_eq!(result.count, 1);
    assert_ne!(
        result.active.unwrap().start.line,
        result.active.unwrap().end.line
    );
    assert_eq!(finish(&e, "fg\nlast", SearchDirection::Next, None).count, 1);
    assert_eq!(finish(&e, "fglast", SearchDirection::Next, None).count, 0);
}
#[test]
fn overlaps_next_previous_wrap_and_no_match_limit() {
    let mut e = Emulator::new(TermSize::new(80, 5, 0, 0), EmulatorOptions::default());
    e.advance(format!("{}\r\n", "a".repeat(5_000)).as_bytes());
    let first = finish(&e, "aa", SearchDirection::Next, None);
    assert_eq!(first.count, 4_999);
    let second = finish(
        &e,
        "aa",
        SearchDirection::Next,
        first.active.map(|m| m.start),
    );
    assert_eq!(second.ordinal, 2);
    let last = finish(
        &e,
        "aa",
        SearchDirection::Previous,
        first.active.map(|m| m.start),
    );
    assert_eq!(last.ordinal, 4_999);
    let wrapped = finish(
        &e,
        "aa",
        SearchDirection::Next,
        last.active.map(|m| m.start),
    );
    assert_eq!(wrapped.active, first.active);
}
#[test]
fn bounded_steps_invalidation_empty_and_query_limit() {
    let mut e = Emulator::new(TermSize::default(), EmulatorOptions::default());
    e.advance(b"one\r\ntwo");
    assert!(e.search("", SearchDirection::Next, None).is_err());
    assert!(
        e.search(
            &"a".repeat(MAX_SEARCH_QUERY + 1),
            SearchDirection::Next,
            None
        )
        .is_err()
    );
    let mut scan = e.search("two", SearchDirection::Next, None).unwrap();
    assert_eq!(scan.step(&e, 1), SearchProgress::Searching);
    e.advance(b"changed");
    assert_eq!(scan.step(&e, 1), SearchProgress::Invalidated);
}

#[test]
fn search_highlight_never_changes_selection_and_history_eviction_invalidates() {
    use crate::{CellPoint, Frame, SelectionKind, Side};
    let mut e = Emulator::new(TermSize::new(12, 2, 0, 0), EmulatorOptions::default());
    e.advance(b"pick needle\r\nnext\r\nlast");
    let result = finish(&e, "needle", SearchDirection::Next, None);
    assert!(
        result.active.unwrap().start.line < 0,
        "retained history is searched"
    );
    e.show_search_match(result.active.unwrap());
    e.start_selection(
        SelectionKind::Cells,
        CellPoint { row: 0, col: 0 },
        Side::Left,
    );
    e.update_selection(CellPoint { row: 0, col: 3 }, Side::Right);
    assert_eq!(e.selection_text().as_deref(), Some("pick"));
    let mut frame = Frame::default();
    e.snapshot(&mut frame);
    assert_eq!(frame.cells.iter().filter(|c| c.selected).count(), 4);
    assert_eq!(frame.cells.iter().filter(|c| c.search_hit).count(), 6);
    e.clear_selection();
    e.snapshot(&mut frame);
    assert!(!frame.cells.iter().any(|c| c.selected));
    assert!(frame.cells.iter().any(|c| c.search_hit));
    let mut scan = e.search("needle", SearchDirection::Next, None).unwrap();
    e.set_options(EmulatorOptions {
        scrollback_lines: 0,
        ..Default::default()
    });
    assert_eq!(scan.step(&e, 1), SearchProgress::Invalidated);
    assert_eq!(finish(&e, "needle", SearchDirection::Next, None).count, 0);
}

#[test]
fn resize_reflow_and_alternate_screen_do_not_keep_stale_matches() {
    use crate::Frame;
    let mut e = Emulator::new(TermSize::new(8, 4, 0, 0), EmulatorOptions::default());
    e.advance(b"literal[.*]needle");
    let result = finish(&e, "[.*]needle", SearchDirection::Next, None);
    assert_eq!(result.count, 1);
    e.show_search_match(result.active.unwrap());
    let mut scan = e.search("needle", SearchDirection::Next, None).unwrap();
    e.resize(TermSize::new(16, 4, 0, 0));
    assert_eq!(scan.step(&e, 1), SearchProgress::Invalidated);
    assert_eq!(
        finish(&e, "[.*]needle", SearchDirection::Next, None).count,
        1
    );
    let mut frame = Frame::default();
    e.snapshot(&mut frame);
    assert!(!frame.cells.iter().any(|c| c.search_hit));
    e.advance(b"\x1b[?1049halternate");
    assert_eq!(finish(&e, "needle", SearchDirection::Next, None).count, 0);
    e.advance(b"\x1b[?1049l");
    assert_eq!(finish(&e, "needle", SearchDirection::Next, None).count, 1);
}

#[test]
fn many_combining_scalars_use_bounded_steps_and_distinct_overlap_anchors() {
    let mut e = Emulator::new(TermSize::new(100, 2, 0, 0), EmulatorOptions::default());
    // Keep thousands of scalars under tiny scheduler budgets while respecting
    // the engine's 64-mark cap per cell.
    let cluster = format!("a{}", "\u{301}".repeat(50));
    e.advance(cluster.repeat(100).as_bytes());
    let mut scan = e
        .search("\u{301}\u{301}", SearchDirection::Next, None)
        .unwrap();
    for _ in 0..100 {
        assert_eq!(scan.step(&e, 1), SearchProgress::Searching);
    }
    let first = finish(&e, "\u{301}\u{301}", SearchDirection::Next, None);
    assert_eq!(first.count, 100 * 49);
    let second = finish(
        &e,
        "\u{301}\u{301}",
        SearchDirection::Next,
        first.active.map(|m| m.start),
    );
    assert_eq!(second.ordinal, 2);
    assert_eq!(
        first.active.unwrap().start.column,
        second.active.unwrap().start.column
    );
    assert_ne!(first.active.unwrap().start, second.active.unwrap().start);
}
