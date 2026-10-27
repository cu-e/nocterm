//! A restored screen replaces what the program drew; it is not output.
use super::*;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use nocterm_session::{Auth, Target};
use std::{cell::Cell, rc::Rc};

struct Fixture {
    terminal: Entity<Terminal>,
    bells: Rc<Cell<usize>>,
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    let bells = Rc::new(Cell::new(0));
    let seen = bells.clone();
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
        transport.enqueue_session(nocterm_session::channel(None).0);
        let installed = crate::install(
            crate::Deps::new(
                transport,
                crate::LocalSessions::disabled("/bin/bash".into()),
            ),
            &ui,
            cx,
        );
        cx.new(|cx| {
            cx.subscribe_self(move |_, event: &TerminalEvent, _| {
                if matches!(event, TerminalEvent::Bell) {
                    seen.set(seen.get() + 1);
                }
            })
            .detach();
            let spec = SessionSpec {
                profile: None,
                options: Default::default(),
                title: "test".into(),
                target: Target::new("me", "host", 22),
                auth: Auth::Password,
                launch: None,
                credential: None,
                jumps: Vec::new(),
            };
            Terminal::new(installed, spec, cx)
        })
    });
    Fixture { terminal, bells }
}

fn rows(terminal: &Terminal) -> Vec<String> {
    let mut frame = nocterm_vt::Frame::default();
    terminal.emulator().snapshot(&mut frame);
    (0..frame.size.rows)
        .map(|row| frame.row_text(row))
        .collect()
}

#[gpui_kit::test]
fn restore_leaves_only_the_picture_without_old_rows_history_or_modes(cx: &mut TestAppContext) {
    let Fixture { terminal, .. } = fixture(cx);
    // The picture does not clear the screen: whatever else is gone, the restore removed.
    let picture = b"prompt$ ls";
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            let history = (0..60)
                .map(|n| format!("old line {n}\r\n"))
                .collect::<String>();
            let old = format!("{history}older\r\nscreen\x1b[?1000h\x1b[?1006h\x1b]0;old title\x07");
            terminal.handle_event(Event::Output(old.into_bytes()), cx);
            assert!(terminal.emulator().modes().mouse_clicks);
            let before = terminal.emulator().generation();

            terminal.handle_event(Event::Restore(picture.to_vec()), cx);

            let rows = rows(terminal);
            assert_eq!(rows[0], "prompt$ ls");
            assert!(
                rows[1..].iter().all(|row| row.trim().is_empty()),
                "{rows:?}"
            );
            terminal.scroll(nocterm_vt::Scroll::Top, cx);
            assert_eq!(
                self::rows(terminal),
                rows,
                "no history survives above the picture"
            );
            let modes = terminal.emulator().modes();
            assert!(
                !modes.mouse_clicks && !modes.mouse_sgr,
                "the old modes are gone"
            );
            assert_eq!(terminal.program_title(), None, "the old title is gone");
            assert!(
                terminal.emulator().generation() > before,
                "caches keyed by the generation must miss after a restore"
            );
        })
    });
}

#[gpui_kit::test]
fn restore_paints_what_a_fresh_emulator_paints_and_keeps_the_picture_title(
    cx: &mut TestAppContext,
) {
    let Fixture { terminal, .. } = fixture(cx);
    let picture = b"\x1b]0;shell\x07prompt$ ls\r\nfile-a  file-b\r\n\x1b[31mred\x1b[0m";
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            terminal.handle_event(Event::Output(b"stale\r\nrows".to_vec()), cx);
            terminal.handle_event(Event::Restore(picture.to_vec()), cx);

            let mut fresh = Emulator::new(
                terminal.emulator().size(),
                emulator_options(
                    terminal
                        .installed()
                        .settings()
                        .setting::<TerminalSettings>(cx),
                ),
            );
            fresh.advance(picture);
            let (mut restored, mut expected) =
                (nocterm_vt::Frame::default(), nocterm_vt::Frame::default());
            terminal.emulator().snapshot(&mut restored);
            fresh.snapshot(&mut expected);
            assert_eq!(restored.cells, expected.cells);
            assert_eq!(terminal.program_title(), Some("shell"));
        })
    });
}

#[gpui_kit::test]
fn a_restored_picture_rings_no_bell(cx: &mut TestAppContext) {
    let Fixture { terminal, bells } = fixture(cx);
    let send = |cx: &mut TestAppContext, event: Event| {
        cx.update(|cx| terminal.update(cx, |terminal, cx| terminal.handle_event(event, cx)));
    };
    send(cx, Event::Connected);
    send(cx, Event::Output(b"\x07".to_vec()));
    assert_eq!(bells.get(), 1, "the program's own bell still rings");
    send(cx, Event::Restore(b"picture\x07".to_vec()));
    assert_eq!(bells.get(), 1);
}

#[gpui_kit::test]
fn a_restored_screen_forgets_the_directory_the_shell_reported(cx: &mut TestAppContext) {
    let Fixture { terminal, .. } = fixture(cx);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            terminal.handle_event(Event::Output(b"\x1b]7;file:///tmp/work\x07".to_vec()), cx);
            assert_eq!(terminal.cwd(), Some("/tmp/work".into()));

            terminal.handle_event(Event::Restore(b"prompt$ ".to_vec()), cx);

            assert_eq!(
                terminal.cwd(),
                None,
                "the picture does not say where the shell is"
            );
        })
    });
}
