use super::*;
use gpui_kit::px;

fn commands(f: &Fixture, values: Vec<acp::AvailableCommand>, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.current().unwrap().update(cx, |thread, cx| {
                thread.state.commands = values;
                cx.notify();
            });
            panel
                .input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn clicked_unicode_command_keeps_focus_and_appends_at_end(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    commands(
        &f,
        vec![acp::AvailableCommand::new("обзор", "Review changes")],
        "/",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("slash-command", 0usize), cx);
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.input("код", cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/обзор код"
        );
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn long_description_has_same_row_height_and_bounded_full_details(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    let long = "First paragraph\n\n## Instructions\n\n".to_string()
        + &"Extensive description with examples.\n".repeat(150);
    commands(
        &f,
        vec![
            acp::AvailableCommand::new("short", "Short description"),
            acp::AvailableCommand::new("long", long),
        ],
        "/",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let short = window
            .find(("command-summary", 0usize))
            .bounds()
            .size
            .height;
        let long = window
            .find(("command-summary", 1usize))
            .bounds()
            .size
            .height;
        assert!(
            (short - long).abs() <= px(1.),
            "summary wrapping grew row: {short:?}, {long:?}"
        );
        assert!(short >= px(35.) && short < px(90.), "row padding {short:?}");
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "down");
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("command-details").is_none());
        window.hover(("command-summary", 1usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let description = window.find("command-description").bounds().size;
        assert!(description.height <= px(180.));
        assert!(description.height >= px(100.));
        assert!(description.width < px(400.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn typed_command_is_highlighted_and_details_only_appear_on_token_hover(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    commands(
        &f,
        vec![acp::AvailableCommand::new("long", "Full command details")],
        "/long arguments",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("command-details").is_none());
        assert!(window.try_find("agent-slash-commands").is_none());
        assert!(window.try_find("composer-command-token").is_some());
        window.hover("composer-command-token", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("command-details").is_some());
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/long arguments"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn arrows_before_tab_and_tab_cycle_reveal_offscreen_commands(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    commands(
        &f,
        (0..20)
            .map(|i| acp::AvailableCommand::new(format!("command{i}"), "Description"))
            .collect(),
        "/",
        cx,
    );
    for _ in 0..12 {
        cx.simulate_keystrokes(f.handle, "down");
        cx.run_until_parked();
    }
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = f.panel.read(cx);
        assert_eq!(panel.commands.selected, 12);
        assert_eq!(panel.input.read(cx).value().as_ref(), "/");
        assert!(panel.input.read(cx).focus_handle(cx).is_focused(window));
        let bounds = panel.commands.scroll.bounds_for_item(12).unwrap();
        let visible = panel.commands.scroll.viewport_bounds();
        assert!(bounds.top() >= visible.top());
        assert!(bounds.bottom() <= visible.bottom());
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "tab tab tab");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = f.panel.read(cx);
        assert_eq!(panel.commands.selected, 14);
        assert_eq!(panel.input.read(cx).value().as_ref(), "/command14 ");
        let bounds = panel.commands.scroll.bounds_for_item(14).unwrap();
        let visible = panel.commands.scroll.viewport_bounds();
        assert!(bounds.bottom() <= visible.bottom());
        panel
            .input
            .clone()
            .update(cx, |input, cx| input.set_selected_range(0..0, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(f.panel.read(cx).commands.selected, 14));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .read(cx)
            .input
            .clone()
            .update(cx, |input, cx| input.set_value("/command0", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(f.panel.read(cx).commands.selected, 0);
        assert_eq!(
            f.panel
                .read(cx)
                .commands
                .scroll
                .logical_scroll_top()
                .item_ix,
            0
        );
        assert_eq!(
            f.panel
                .read(cx)
                .commands
                .scroll
                .logical_scroll_top()
                .offset_in_item,
            px(0.)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn pasted_command_annotation_preserves_caret_undo_delete_and_raw_submission(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    commands(
        &f,
        vec![acp::AvailableCommand::new("обзор", "Review")],
        "",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.input("/обзор аргументы", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let input = f.panel.read(cx).input.read(cx);
        assert_eq!(input.tokens().len(), 1);
        assert_eq!(
            input.selected_range(),
            "/обзор аргументы".len().."/обзор аргументы".len()
        );
        window.input("!", cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/обзор аргументы!"
        );
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "ctrl-z ctrl-z ctrl-z");
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(f.panel.read(cx).input.read(cx).value().as_ref(), ""));
    cx.update_window(f.handle, |_, window, cx| {
        window.input("/обзор аргументы", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("composer-command-token", cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).selected_range(),
            0.."/обзор".len()
        );
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "backspace");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            " аргументы"
        )
    });
    cx.simulate_keystrokes(f.handle, "ctrl-z");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        let input = f.panel.read(cx).input.clone();
        input.update(cx, |input, cx| {
            input.set_selected_range(0..0, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "delete");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            " аргументы"
        )
    });
    cx.simulate_keystrokes(f.handle, "ctrl-z");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.send(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(
        matches!(&prompts[0].prompt[0], acp::ContentBlock::Text(text) if text.text == "/обзор аргументы")
    );
}

#[gpui_kit::test]
fn model_token_hover_opens_actual_model_picker(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                let mut option = acp::SessionConfigOption::select(
                    "model",
                    "Model",
                    "one",
                    vec![
                        acp::SessionConfigSelectOption::new("one", "First"),
                        acp::SessionConfigSelectOption::new("two", "Second"),
                    ],
                );
                option.category = Some(acp::SessionConfigOptionCategory::Model);
                thread.state.config_options = vec![option];
                cx.notify();
            });
    });
    commands(
        &f,
        vec![acp::AvailableCommand::new("model", "Choose a model")],
        "/model ",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.hover("composer-command-token", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("command-browse-models", cx);
        window.render_frame(cx);
        assert!(f.panel.read(cx).menu == Some(crate::panel::MenuKind::Config("model".into())));
        assert!(window.try_find("agent-picker").is_some());
        assert!(window.try_find("config-model-two").is_some());
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn clipboard_paste_keeps_the_command_and_arguments_editable(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    commands(
        &f,
        vec![acp::AvailableCommand::new("обзор", "Review changes")],
        "",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "/обзор изменения".into(),
        ));
        window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let input = f.panel.read(cx).input.read(cx);
        assert_eq!(input.value().as_ref(), "/обзор изменения");
        assert_eq!(input.tokens()[0].range(), 0.."/обзор".len());
        assert_eq!(
            input.selected_range(),
            "/обзор изменения".len().."/обзор изменения".len()
        );
        assert!(window.try_find("command-details").is_none());
        window.input("!", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/обзор изменения!"
        )
    });
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn typing_slash_then_arrow_selects_before_tab_without_moving_the_caret(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    commands(
        &f,
        vec![
            acp::AvailableCommand::new("first", "First action"),
            acp::AvailableCommand::new("second", "Second action"),
        ],
        "",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.input("/", cx);
        window.render_frame(cx);
        window.press("up", cx);
        assert_eq!(f.panel.read(cx).commands.selected, 1);
        assert_eq!(f.panel.read(cx).input.read(cx).selected_range(), 1..1);
        window.press("down", cx);
        assert_eq!(f.panel.read(cx).commands.selected, 0);
        window.press("down", cx);
        window.press("tab", cx);
        assert_eq!(f.panel.read(cx).input.read(cx).value().as_ref(), "/second ");
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
fn escape_dismisses_whitespace_prefixed_suggestions_then_propagates(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    commands(
        &f,
        vec![acp::AvailableCommand::new("review", "Review changes")],
        " /",
        cx,
    );
    let escaped = Arc::new(AtomicUsize::new(0));
    let observed = escaped.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &gpui_kit::component::input::Escape, _| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_some());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_none());
        assert_eq!(
            escaped.load(Ordering::SeqCst),
            0,
            "first Escape belongs to the open suggestions"
        );
        window.press("escape", cx);
        assert_eq!(
            escaped.load(Ordering::SeqCst),
            1,
            "dismissed suggestions must not swallow Escape"
        );
        f.panel.update(cx, |panel, cx| {
            panel.input.update(cx, |input, cx| {
                input.set_value("/review arguments", window, cx)
            });
        });
        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_none());
        window.press("escape", cx);
        assert_eq!(
            escaped.load(Ordering::SeqCst),
            2,
            "a command with arguments has no open suggestions"
        );
    })
    .unwrap();
}
