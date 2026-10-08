use super::*;

#[gpui_kit::test]
fn usage_card_shows_context_tokens_and_plan_limits(cx: &mut TestAppContext) {
    let f = fixture(cx);
    // Codex writes its limits into its own logs.
    let logs = f._directory.path().join("codex/sessions/2026/10/03");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join("rollout-a.jsonl"),
        r#"{"payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":38.0,"window_minutes":10080,"resets_at":1791588105},"secondary":{"used_percent":12.0,"window_minutes":300,"resets_at":1791500000}}}}"#,
    )
    .unwrap();
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
        thread.read(cx).session().clone().unwrap()
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::UsageUpdate(acp::UsageUpdate::new(25_900, 256_000)),
        )))
        .unwrap();
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish
        .send(
            acp::PromptResponse::new(acp::StopReason::EndTurn)
                .usage(acp::Usage::new(30_000, 25_000, 5_000).cached_read_tokens(20_000)),
        )
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let runtime = Runtime::global(cx).read(cx);
        let limits: Vec<_> = runtime.limits("codex").unwrap().iter().cloned().collect();
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].window, nocterm_ai::usage::Window::FiveHour);
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(
            thread.read(cx).state.tokens.as_ref().unwrap().total_tokens,
            30_000
        );
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-context-usage", cx);
        window.render_frame(cx);
        let card = window.find("agent-usage").bounds();
        let composer = window.find("agent-composer").bounds();
        // Over the chat, above the composer.
        assert!(card.bottom() <= composer.origin.y + gpui_kit::px(1.));
        window.click("agent-usage-close", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn attach_menu_lists_servers_under_their_folders(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let server = |id: &str, group: Option<&str>| nocterm_workspace::ConnectionSummary {
        id: id.to_owned().into(),
        name: id.to_owned().into(),
        group: group.map(|group| group.to_owned().into()),
        description: Default::default(),
        target: serde_json::from_value(
            serde_json::json!({"host":"example.test", "port":22, "user":"user"}),
        )
        .unwrap(),
        icon: None,
        flag: None,
    };
    let directory = Rc::new(Directory(std::cell::RefCell::new(vec![
        server("proxmox", Some("homelab")),
        server("backup", None),
        server("ai-server", Some("homelab")),
    ])));
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(directory.clone())
        })
    });
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        let bounds = |id: &str| window.find(id.to_owned()).bounds();
        let backup = bounds("attach-connection-backup");
        let folder = bounds("attach-group-homelab");
        let ai = bounds("attach-connection-ai-server");
        let proxmox = bounds("attach-connection-proxmox");
        assert!(backup.origin.y < folder.origin.y);
        assert!(folder.origin.y < ai.origin.y && ai.origin.y < proxmox.origin.y);
        // Servers in a folder sit indented under it.
        assert!(ai.origin.x > folder.origin.x + gpui_kit::px(8.));
        assert_eq!(proxmox.origin.x, ai.origin.x);
        assert_eq!(backup.origin.x, folder.origin.x);
        window.click("attach-group-homelab", cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .attachments
                .contains(&Attachment::Group("homelab".into()))
        );
    })
    .unwrap();
}

#[test]
fn implicit_markdown_images_are_disabled_and_config_labels_show_selection() {
    assert_eq!(
        crate::panel::widgets::safe_markdown("![secret](file:///hidden)<img src='x'>"),
        "[secret](file:///hidden)&lt;img src='x'>"
    );
    let option = acp::SessionConfigOption::select(
        "model",
        "Model",
        "one",
        vec![acp::SessionConfigSelectOption::new("one", "First")],
    );
    assert_eq!(crate::thread::config_label(&option), "First");
}
#[gpui_kit::test]
fn image_preparation_runs_once_and_renders_a_cached_small_static_preview(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(800, 800))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let data = png.into_inner();
    let key = (1usize, data.len(), false);
    cx.update(|cx| {
        f.panel.update(cx, |panel, cx| {
            panel.cached_image(key, || data, false, cx);
            assert!(matches!(
                panel.image_cache.get(&key),
                Some(crate::panel::CachedImage::Loading(_))
            ));
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        f.panel.update(cx, |panel, cx| {
            let Some(crate::panel::CachedImage::Ready(image)) = panel.image_cache.get(&key) else {
                panic!("preview not ready");
            };
            let decoded = image::load_from_memory(&image.bytes).unwrap();
            assert!(decoded.width() <= 640 && decoded.height() <= 640);
            panel.cached_image(key, || panic!("cache hit must not decode again"), false, cx);
        })
    });
}

#[gpui_kit::test]
fn agent_popup_is_anchored_to_header_and_dismisses(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-picker").is_none());
        let trigger = window.find("agent-new-thread").bounds();
        window.click("agent-new-thread", cx);
        let picker = window.find("agent-picker").bounds();
        assert!(picker.origin.y >= trigger.bottom());
        for agent in ["codex", "claude", "hermes"] {
            let row = window
                .find(gpui_kit::SharedString::from(format!("new-agent-{agent}")))
                .bounds();
            assert_eq!(row.origin.x, picker.origin.x);
            assert_eq!(row.size.width, picker.size.width);
        }
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-picker").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn composer_popups_select_config_modes_and_context(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    cx.update(|cx| {
        f.panel.read(cx).current().unwrap().update(cx, |thread, cx| {
            thread.state.config_options = vec![acp::SessionConfigOption::select("model", "Model", "one", vec![
                acp::SessionConfigSelectOption::new("one", "Very long selected model name that must fit the narrow panel"),
                acp::SessionConfigSelectOption::new("two", "Second"),
            ])];
            thread.state.modes = Some(serde_json::from_value(serde_json::json!({"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"},{"id":"code","name":"Code"}]})).unwrap());
            cx.notify();
        });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let composer = window.find("agent-composer").bounds();
        let send = window.find("agent-send").bounds();
        assert!(send.right() <= composer.right());
        assert!(send.origin.x >= composer.origin.x);
        let selector = window.find("config-picker-model").bounds();
        assert!(selector.right() <= send.origin.x);
        assert_eq!(window.find("mode-picker").label(), Some("Ask"));
        window.click("config-picker-model", cx);
        let picker = window.find("agent-picker").bounds();
        assert!(picker.bottom() <= selector.origin.y);
        let row = window.find("config-model-two").bounds();
        assert_eq!(row.origin.x, picker.origin.x);
        assert_eq!(row.size.width, picker.size.width);
        window.click("config-model-two", cx);
        assert!(window.try_find("agent-picker").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.click("mode-picker", cx);
        window.click("mode-code", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("mode-picker").label(), Some("Code"));
        window.click("agent-attach-context", cx);
        let attachment = gpui_kit::SharedString::from(format!("attach-{:?}", f.terminal));
        window.click(attachment, cx);
        assert!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .attachments
                .is_empty()
        );
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-picker").is_none());
        window.click("agent-maximize", cx);
        window.render_frame(cx);
        let composer = window.find("agent-composer").bounds();
        let send = window.find("agent-send").bounds();
        assert!(send.right() <= composer.right());
        assert!(send.origin.x >= composer.origin.x);
    })
    .unwrap();
}

#[test]
fn chat_title_and_usage_handle_empty_unicode_unknown_and_overflow() {
    use nocterm_ai::thread::{Entry, ThreadState};
    let mut state = ThreadState {
        title: Some("Provider title".into()),
        ..Default::default()
    };
    let title = crate::thread::chat_title;
    assert_eq!(title(None, &state), "New Agent");
    assert_eq!(title(Some("Mine"), &state), "Mine");
    state.entries.push(Entry::User(vec![acp::ContentBlock::Text(
        acp::TextContent::new("Привет 🌍"),
    )]));
    assert_eq!(title(None, &state), "Provider title");
    assert_eq!(title(Some("  "), &state), "Provider title");
    state.title = None;
    assert_eq!(title(None, &state), "Привет 🌍");
    assert_eq!(crate::panel::usage::usage_fraction(None), 0.);
    for (used, size, expected) in [
        (50, 100, 0.5),
        (120, 100, 1.),
        (100, 0, 0.),
        (85, 100, 0.85),
    ] {
        let usage = serde_json::from_value(serde_json::json!({"used":used,"size":size})).unwrap();
        assert_eq!(crate::panel::usage::usage_fraction(Some(&usage)), expected);
    }
}

#[test]
fn transcript_activity_tracks_only_live_reasoning_and_unfinished_tools() {
    use nocterm_ai::thread::Entry;
    let tool = |status| {
        Entry::Tool(serde_json::from_value(serde_json::json!({"toolCallId":"call","title":"Read terminal","status":status,"kind":"read"})).unwrap())
    };
    let mut entries = vec![
        Entry::Thought("Earlier thought".into()),
        Entry::Agent("Answer".into()),
        Entry::Thought("Live thought".into()),
    ];
    assert!(!crate::panel::widgets::entry_is_live(&entries, 0, true));
    assert!(crate::panel::widgets::entry_is_live(&entries, 2, true));
    assert!(!crate::panel::widgets::entry_is_live(&entries, 2, false));
    entries.push(tool("in_progress"));
    assert!(!crate::panel::widgets::entry_is_live(&entries, 2, true));
    assert!(crate::panel::widgets::entry_is_live(&entries, 3, true));
    for status in ["completed", "failed"] {
        entries[3] = tool(status);
        assert!(!crate::panel::widgets::entry_is_live(&entries, 3, true));
    }
    entries[3] = tool("pending");
    assert!(crate::panel::widgets::entry_is_live(&entries, 3, true));
    assert!(!crate::panel::widgets::entry_is_live(&entries, 3, false));
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn compact_chips_and_live_reasoning_respect_narrow_layout_and_reduced_motion(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                thread.state.entries = vec![
                    nocterm_ai::thread::Entry::User(vec![acp::ContentBlock::Text(
                        acp::TextContent::new("Question"),
                    )]),
                    nocterm_ai::thread::Entry::Thought(
                        "Inspecting the current terminal carefully before running commands".into(),
                    ),
                ];
                thread.generating = true;
                cx.notify();
            });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let chips = window.find("agent-attachment-chips").bounds();
        let chip = window.find("chip-Terminal").bounds();
        assert!(chip.size.width < chips.size.width);
        assert_eq!(chip.origin.x, chips.origin.x);
        let panel = window.find("agent-panel").bounds();
        let bubble = window.find(("agent-user-bubble", 0usize)).bounds();
        assert!(bubble.right() <= panel.right());
        let user_row = window.find(("agent-entry", 0usize)).bounds();
        assert_eq!(bubble.right(), user_row.right() - gpui_kit::px(12.));
        assert!(
            bubble.size.width < user_row.size.width * 0.5,
            "short bubble should fit its content"
        );
        let thought = window.find(("thought", 1usize)).bounds();
        assert!(
            thought.right() <= panel.right(),
            "thought {thought:?} outside panel {panel:?}; entry {:?}",
            window.find(("agent-entry", 1usize)).bounds()
        );
        assert!(
            window.simulate_next_frame(cx) > 0,
            "live reasoning must animate"
        );
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "reduced motion must stop animation"
        );
    })
    .unwrap();
    cx.update(|cx| {
        cx.set_reduce_motion(false);
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                thread.generating = false;
                cx.notify();
            });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "completed reasoning must be static"
        );
        window.click("chip-Terminal", cx);
        assert!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .attachments
                .is_empty()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn expanded_long_mcp_command_json_and_reasoning_stay_within_narrow_panel(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    let long = "very_long_unbroken_argument_".repeat(300);
    cx.update(|cx| {
        f.panel.read(cx).current().unwrap().update(cx, |thread, cx| {
            thread.state.entries = vec![nocterm_ai::thread::Entry::Tool(serde_json::from_value(serde_json::json!({"toolCallId":"mcp","title":format!("terminal.run_command {long}"),"status":"completed","kind":"execute","content":[{"type":"content","content":{"type":"text","text":long}}]})).unwrap())];
            cx.notify();
        });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool-call", 0usize), cx);
        let panel = window.find("agent-panel").bounds();
        for id in ["agent-entry", "tool-call", "tool-output"] {
            let bounds = window.find((id, 0usize)).bounds();
            assert!(
                bounds.right() <= panel.right(),
                "{id}: {bounds:?}, panel {panel:?}"
            );
            assert!(bounds.origin.x >= panel.origin.x);
        }
        // Wrapped, the output is tall enough to push the header out of view.
        f.panel.update(cx, |panel, cx| {
            panel.expanded.clear();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find(("tool-output", 0usize)).is_none());
    })
    .unwrap();
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                thread.state.entries = vec![nocterm_ai::thread::Entry::Thought(format!(
                    "Inspecting command `{long}`\n\n```json\n{{\"command\":\"{long}\"}}\n```"
                ))];
                cx.notify();
            });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("thought", 0usize), cx);
        let panel = window.find("agent-panel").bounds();
        for id in ["agent-entry", "thought", "thought-output"] {
            let bounds = window.find((id, 0usize)).bounds();
            assert!(
                bounds.right() <= panel.right(),
                "{id}: {bounds:?}, panel {panel:?}"
            );
        }
    })
    .unwrap();
}
