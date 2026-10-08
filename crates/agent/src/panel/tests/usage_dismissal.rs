use super::*;
use gpui_kit::{point, px};

fn open_usage(f: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window.click("agent-context-usage", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_some());
    })
    .unwrap();
    cx.run_until_parked();
}

fn settle(f: &Fixture, cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn usage_escape_dismisses_only_the_card_then_restores_composer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let escaped = Arc::new(AtomicUsize::new(0));
    let observed = escaped.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &gpui_kit::component::input::Escape, _| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
    });
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        assert_eq!(escaped.load(Ordering::SeqCst), 0);
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.input("draft after closing", cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "draft after closing"
        );
        window.press("escape", cx);
        assert_eq!(escaped.load(Ordering::SeqCst), 1);
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn usage_escape_precedes_slash_suggestions(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.current().unwrap().update(cx, |thread, cx| {
                thread.state.commands = vec![acp::AvailableCommand::new("review", "Review")];
                cx.notify();
            });
            panel
                .input
                .update(cx, |input, cx| input.set_value("/", window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert!(f.panel.read(cx).commands.dismissed.is_none());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        assert!(window.try_find("agent-slash-commands").is_some());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_none());
        assert_eq!(f.panel.read(cx).input.read(cx).value().as_ref(), "/");
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_composer_click_dismisses_and_preserves_the_click_and_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click_at("agent-composer", point(px(20.), px(20.)), cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.input("kept draft", cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "kept draft"
        );
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn usage_focus_leaving_the_panel_dismisses_without_stealing_focus(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let terminal = f.workspace.read(cx).find_item::<FakeTerminal>().unwrap();
        let focus = terminal.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let terminal = f.workspace.read(cx).find_item::<FakeTerminal>().unwrap();
        let focus = terminal.read(cx).focus_handle(cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        assert!(focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_inside_click_and_tab_keep_card_open_until_focus_leaves(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click_at("agent-usage", point(px(20.), px(55.)), cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_some());
        window.press("tab", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_some());
        let input_focus = f.panel.read(cx).input.read(cx).focus_handle(cx);
        window.focus(&input_focus, cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let input_focus = f.panel.read(cx).input.read(cx).focus_handle(cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        assert!(input_focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_trigger_toggle_and_outside_menu_action_do_not_reopen_card(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("agent-context-usage", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        window.click("agent-context-usage", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_some());
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        assert!(
            window.try_find("agent-picker").is_some(),
            "outside action must still open its own menu"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_stays_above_a_multiline_composer_in_a_narrow_panel(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 14.);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let input = f.panel.read(cx).input.clone();
        input.update(cx, |input, cx| {
            input.set_value("A draft line\n".repeat(10), window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    settle(&f, cx);
    open_usage(&f, cx);
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let card = window.find("agent-usage").bounds();
        let composer = window.find("agent-composer").bounds();
        assert!(card.bottom() <= composer.top() + px(1.));
        assert!(card.left() >= px(0.) && card.right() <= window.viewport_size().width);
        assert!(card.top() >= px(0.) && card.bottom() <= window.viewport_size().height);
        window.click_at("agent-composer", point(px(20.), px(20.)), cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert!(window.try_find("agent-usage").is_none());
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "A draft line\n".repeat(10)
        );
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_keyboard_tab_leaving_the_card_dismisses_without_trapping_focus(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click_at("agent-usage", point(px(20.), px(55.)), cx);
        window.render_frame(cx);
        window.press("tab", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_some());
        window.press("tab", cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert!(window.try_find("agent-usage").is_none());
        assert!(!f.panel.read(cx).usage_focus.contains_focused(window, cx));
        assert!(window.focused(cx).is_some());
    })
    .unwrap();
}

fn assert_original_usage_bounds(window: &Window) {
    let card = window.find("agent-usage").bounds();
    let chat = window.find("agent-transcript-area").bounds();
    let composer = window.find("agent-composer").bounds();
    let inset = window.rem_size() * 0.75;
    assert!(
        (card.left() - (chat.left() + inset)).abs() <= px(0.5),
        "card left must retain the original chat inset"
    );
    assert!(
        (card.right() - (chat.right() - inset)).abs() <= px(0.5),
        "card right must retain the original chat inset"
    );
    assert!(
        (card.bottom() - (chat.bottom() - window.rem_size() * 0.25)).abs() <= px(0.5),
        "card must retain its original bottom position"
    );
    assert!(card.bottom() <= composer.top());
}

#[gpui_kit::test]
fn usage_retains_original_full_chat_width_in_a_wide_panel(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 40.);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        assert_original_usage_bounds(window)
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_retains_original_full_chat_width_in_a_narrow_panel(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 14.);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        assert_original_usage_bounds(window)
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_original_bounds_follow_window_resize(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(true, cx)
        });
    });
    settle(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1100.), px(900.)));
        window.render_frame(cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        assert_original_usage_bounds(window)
    })
    .unwrap();
}

#[gpui_kit::test]
fn usage_closes_when_typing_starts_without_requiring_a_composer_click(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    open_usage(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.input("Пишу черновик", cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "Пишу черновик"
        );
        assert!(window.try_find("agent-usage").is_none());
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn usage_trigger_remains_keyboard_accessible(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        for _ in 0..30 {
            if window.find("agent-context-usage").focused() == Some(true) {
                break;
            }
            window.focus_next(cx);
            window.render_frame(cx);
        }
        assert_eq!(window.find("agent-context-usage").focused(), Some(true));
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_some());
        assert_original_usage_bounds(window);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}
