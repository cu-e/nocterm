//! Mouse negotiation identity must survive ordinary host viewport work.
use super::*;

fn emulator() -> Emulator {
    Emulator::new(TermSize::new(20, 3, 8, 16), EmulatorOptions::default())
}

#[test]
fn tracking_round_trip_changes_identity_even_when_final_flags_match() {
    let mut terminal = emulator();
    terminal.advance(b"\x1b[?1002h");
    let before = terminal.modes();
    terminal.advance(b"\x1b[?1002l\x1b[?1002h");
    let after = terminal.modes();
    assert!(after.mouse_drag && before.mouse_drag);
    assert_ne!(after.mouse_tracking_epoch, before.mouse_tracking_epoch);
}

#[test]
fn host_scroll_events_do_not_change_tracking_identity_on_next_output() {
    let mut terminal = emulator();
    terminal.advance(b"1\r\n2\r\n3\r\n4\r\n5\r\n\x1b[?1002h");
    let epoch = terminal.modes().mouse_tracking_epoch;
    assert!(terminal.scroll(Scroll::Lines(1)));
    terminal.advance(b"ordinary output");
    assert_eq!(terminal.modes().mouse_tracking_epoch, epoch);
    // A real negotiation queued after that host event still changes identity.
    assert!(terminal.scroll(Scroll::Lines(1)));
    terminal.advance(b"\x1b[?1002l\x1b[?1002h");
    assert_ne!(terminal.modes().mouse_tracking_epoch, epoch);
}

#[test]
fn synchronized_tracking_reset_changes_identity_only_when_applied() {
    let mut terminal = emulator();
    terminal.advance(b"\x1b[?1003h");
    let epoch = terminal.modes().mouse_tracking_epoch;
    terminal.advance(b"\x1b[?2026h\x1b[?1003l\x1b[?1003h");
    assert!(terminal.sync_deadline().is_some());
    assert_eq!(terminal.modes().mouse_tracking_epoch, epoch);
    terminal.finish_sync();
    assert!(terminal.modes().mouse_motion);
    assert_ne!(terminal.modes().mouse_tracking_epoch, epoch);
}

#[test]
fn empty_drag_anchor_remains_owned_until_selection_is_cleared() {
    let mut terminal = emulator();
    terminal.start_selection(SelectionKind::Cells, CellPoint::default(), Side::Left);
    assert!(terminal.has_selection());
    assert!(terminal.selection_start().is_none());
    assert!(terminal.selection_text().is_none());
    assert!(terminal.prepare_input());
    assert!(!terminal.has_selection());
}
