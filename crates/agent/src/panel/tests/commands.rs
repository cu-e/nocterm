use super::*;
use gpui_kit::px;

fn commands(f: &Fixture, values: Vec<acp::AvailableCommand>, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.current().unwrap().update(cx, |thread, cx| {
                report_control(
                    thread,
                    acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
                        values,
                    )),
                    cx,
                );
                cx.notify();
            });
        });
        replace_composer_value(f.panel.read(cx).composer_focus(cx), text, window, cx);

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
        assert!(f.panel.read(cx).composer_focus(cx).is_focused(window));
        window.input("код", cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/обзор код");
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn long_description_has_same_row_height_and_bounded_full_details(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., usage_source(), workspace_resolver(), image_deps());
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
            f.panel.read(cx).composer_text(cx).as_ref(),
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
        assert_eq!(
            window.find(("command-hover", 12usize)).selected(),
            Some(true)
        );
        assert_eq!(panel.composer_text(cx).as_ref(), "/");
        assert!(panel.composer_focus(cx).is_focused(window));
        let bounds = window.find(("command-hover", 12usize)).bounds();
        let visible = window.find("agent-slash-commands").bounds();
        assert!(bounds.top() >= visible.top());
        assert!(bounds.bottom() <= visible.bottom());
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "tab tab tab");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = f.panel.read(cx);
        assert_eq!(
            window.find(("command-hover", 14usize)).selected(),
            Some(true)
        );
        assert_eq!(panel.composer_text(cx).as_ref(), "/command14 ");
        let bounds = window.find(("command-hover", 14usize)).bounds();
        let visible = window.find("agent-slash-commands").bounds();
        assert!(bounds.bottom() <= visible.bottom());
        window.press("home", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("command-hover", 14usize)).selected(),
            Some(true)
        );
    })
    .unwrap();
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(f.panel.read(cx).composer_focus(cx), "/command0", window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/command0");
        assert_eq!(
            window.find(("command-hover", 0usize)).selected(),
            Some(true)
        );
        let row = window.find(("command-hover", 0usize)).bounds();
        let viewport = window.find("agent-slash-commands").bounds();
        assert!(
            (row.top() - viewport.top() - px(4.)).abs() <= px(1.),
            "first command must align with menu padding"
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
        assert!(window.try_find("composer-command-token").is_some());
        // Appending below proves the native caret stayed at the original UTF-8 end.
        window.input("!", cx);
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "/обзор аргументы!"
        );
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "ctrl-z ctrl-z ctrl-z");
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), ""));
    cx.update_window(f.handle, |_, window, cx| {
        window.input("/обзор аргументы", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("composer-command-token", cx);
        // The following Backspace must remove exactly the clicked command prefix.
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "backspace");
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), " аргументы"));
    cx.simulate_keystrokes(f.handle, "ctrl-z");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.press("home", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "delete");
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), " аргументы"));
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
                report_control(
                    thread,
                    acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(vec![
                        option,
                    ])),
                    cx,
                );
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
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "/обзор изменения"
        );
        assert!(window.try_find("composer-command-token").is_some());
        // The next insertion proves end-caret placement without an Input entity.
        assert!(window.try_find("command-details").is_none());
        window.input("!", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
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
        window.render_frame(cx);
        assert_eq!(
            window.find(("command-hover", 1usize)).selected(),
            Some(true)
        );
        window.input("?", cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/?");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.press("backspace", cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.press("up", cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("command-hover", 0usize)).selected(),
            Some(true)
        );
        window.press("down", cx);
        window.press("tab", cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/second ");
        assert!(f.panel.read(cx).composer_focus(cx).is_focused(window));
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
        replace_composer_value(
            f.panel.read(cx).composer_focus(cx),
            "/review arguments",
            window,
            cx,
        );

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

#[gpui_kit::test]
fn advertised_command_removal_preserves_raw_unicode_caret_and_arguments(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        report_control(
            thread,
            acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(vec![
                acp::AvailableCommand::new("обзор", "Review"),
            ])),
            cx,
        );

        cx.notify();
    });
    set_input(&f, "/обзор args", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.press("left", cx);
        window.press("left", cx);
        window.render_frame(cx);
        assert!(window.try_find("composer-command-token").is_some());
    })
    .unwrap();
    thread.update(cx, |thread, cx| {
        report_control(
            thread,
            acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
                Vec::new(),
            )),
            cx,
        );
        cx.notify();
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/обзор args");
        assert!(window.try_find("composer-command-token").is_none());
        window.input("界", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/обзор ar界gs"));
}

#[gpui_kit::test]
fn thousand_skill_menu_virtualizes_and_wraps_to_visible_variable_height_row(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.panel.update(cx, |panel, cx| panel.current().unwrap().update(cx, |thread, cx| {
        report_control(thread, acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new((0..1000).map(|index| {
            let mut value = serde_json::json!({"name":format!("skill{index:04}"), "description":"description ".repeat(1000)});
            if index % 2 == 1 { value["input"] = serde_json::json!({"hint":"Provide optional arguments"}); }
            serde_json::from_value(value).unwrap()
        }).collect())), cx);

        cx.notify();
    }));
    set_input(&f, "", cx);
    cx.update_window(f.handle, |_, window, cx| window.input("/", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("slash-command", 0usize)).is_some());
        assert!(window.try_find(("slash-command", 999usize)).is_none());
        let first = window.find(("slash-command", 0usize)).bounds();
        let second = window.find(("slash-command", 1usize)).bounds();
        assert!(
            second.size.height > first.size.height,
            "optional hint must change row height"
        );
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("command-hover", 999usize)).selected(),
            Some(true)
        );
        let row = window.find(("slash-command", 999usize)).bounds();
        let viewport = window.find("agent-slash-commands").bounds();
        assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom());
        window.input("?", cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/?");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("command-hover", 999usize)).selected(),
            Some(true)
        );
        assert!(f.panel.read(cx).composer_focus(cx).is_focused(window));
        window.press("tab", cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/skill0999 ");
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/skill0000 ");
        assert!(window.try_find(("slash-command", 0usize)).is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn slash_escape_and_tab_work_through_real_keyboard_events(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.current().unwrap().update(cx, |thread, cx| {
                report_control(
                    thread,
                    acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
                        vec![
                            acp::AvailableCommand::new("review", "Review changes"),
                            acp::AvailableCommand::new("rename", "Rename chat"),
                        ],
                    )),
                    cx,
                );
                cx.notify();
            });
        });
        replace_composer_value(f.panel.read(cx).composer_focus(cx), "/", window, cx);

        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_some());
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "escape");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_none());
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/");
        replace_composer_value(f.panel.read(cx).composer_focus(cx), "/r", window, cx);

        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "tab");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/review ");
        assert!(f.panel.read(cx).composer_focus(cx).is_focused(window));
        window.render_frame(cx);
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
    cx.simulate_keystrokes(f.handle, "shift-tab");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/rename ");
        assert!(f.panel.read(cx).composer_focus(cx).is_focused(window));
        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "tab enter");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/review ");
        assert!(f.panel.read(cx).composer_focus(cx).is_focused(window));
        window.input("these changes", cx);
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "/review these changes"
        );
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}
