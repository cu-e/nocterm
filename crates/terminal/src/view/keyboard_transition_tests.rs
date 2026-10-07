//! Protocol ownership checks use GPUI's mock TestAppContext exclusively.
use super::*;
use gpui_kit::{InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke};

fn press_up(window: &mut Window, cx: &mut App) {
    window.dispatch_event(
        KeyDownEvent {
            keystroke: Keystroke::parse("up").unwrap(),
            is_held: false,
            prefer_character_input: false,
        }
        .to_platform_input(),
        cx,
    );
}

fn release_up(window: &mut Window, cx: &mut App) {
    window.dispatch_event(
        KeyUpEvent {
            keystroke: Keystroke::parse("up").unwrap(),
        }
        .to_platform_input(),
        cx,
    );
}

#[gpui_kit::test]
fn same_effective_flags_still_discard_prior_protocol_ownership(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    for sequence in [
        "\x1b[=1u\x1b[=3u",
        "\x1b[>3u\x1b[<1u",
        "\x1b[=3u",
        "\x1b[?1049h\x1b[=3u\x1b[?1049l",
        "\x1bc\x1b[=3u",
    ] {
        // Keep a saved entry so the nested pop does not empty the stack.
        emit(cx, &transport, 0, Event::Output(b"\x1b[>3u".to_vec()));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            press_up(window, cx);
        })
        .unwrap();
        assert_eq!(drain(&driver), [b"\x1b[A".to_vec()]);
        emit(
            cx,
            &transport,
            0,
            Event::Output(sequence.as_bytes().to_vec()),
        );
        assert_eq!(
            view.read_with(cx, |view, cx| {
                view.terminal
                    .read(cx)
                    .emulator()
                    .modes()
                    .keyboard
                    .kitty_flags()
            }),
            3,
            "{sequence:?} restores the same effective flags"
        );
        cx.update_window(handle, |_, window, cx| release_up(window, cx))
            .unwrap();
        assert!(
            drain(&driver).is_empty(),
            "{sequence:?} must not release a key owned by the earlier context"
        );
    }
}

#[gpui_kit::test]
fn ordinary_output_and_protocol_queries_preserve_owned_releases(cx: &mut TestAppContext) {
    let (handle, _, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    emit(cx, &transport, 0, Event::Output(b"\x1b[>3u".to_vec()));
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        press_up(window, cx);
    })
    .unwrap();
    assert_eq!(drain(&driver), [b"\x1b[A".to_vec()]);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"ordinary output\r\n\x1b[?u".to_vec()),
    );
    assert_eq!(drain(&driver), [b"\x1b[?3u".to_vec()]);
    cx.update_window(handle, |_, window, cx| release_up(window, cx))
        .unwrap();
    assert_eq!(drain(&driver), [b"\x1b[1;1:3A".to_vec()]);
}
