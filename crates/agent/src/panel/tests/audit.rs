use super::*;
use nocterm_ai::history::{SavedAttachment, SavedChat, SavedPrompt};
use nocterm_ai::thread::Entry;

fn set_input(f: &Fixture, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel
                .input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        });
        window.render_frame(cx);
    })
    .unwrap();
}
fn pending_permission(thread: &Entity<crate::thread::AgentThread>, cx: &mut TestAppContext) {
    let (send, _) = nocterm_ai::PermissionResponder::channel();
    thread.update(cx, |thread, cx| {
        let request = serde_json::from_value(serde_json::json!({
            "sessionId":thread.session().as_ref().unwrap(),
            "toolCall":{"toolCallId":"pending","title":"Needs approval"},
            "options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]
        }))
        .unwrap();
        thread.permission(request, send, cx);
    });
}

#[gpui_kit::test]
fn approval_switch_restores_each_draft_and_queue_edit_defaults(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    set_input(&f, "owner draft", cx);
    pending_permission(&owner, cx);
    new_chat(&f, cx);
    let editing = cx.update(|cx| f.panel.read(cx).current().unwrap());
    editing.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.attachments = vec![Attachment::Group("queued scope".into())];
        thread.send("queued".into(), cx);
        thread.attachments = vec![Attachment::Group("draft scope".into())];
    });
    cx.run_until_parked();
    set_input(&f, "editing chat draft", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("queue-edit-first", cx);
        f.panel.update(cx, |panel, cx| panel.review_pending(cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert_eq!(panel.current().unwrap().entity_id(), owner.entity_id());
        assert_eq!(panel.input.read(cx).value().as_ref(), "owner draft");
        assert!(panel.composer.edit.is_none());
        assert!(!editing.read(cx).queue_editing);
        assert_eq!(
            editing.read(cx).attachments,
            vec![Attachment::Group("draft scope".into())]
        );
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(editing.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "editing chat draft"
        )
    });
    complete_active(&f, cx);
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn delete_edit_owner_then_review_does_not_keep_dangling_edit(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    pending_permission(&owner, cx);
    new_chat(&f, cx);
    let editing = cx.update(|cx| f.panel.read(cx).current().unwrap());
    editing.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("queued".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
        f.panel.update(cx, |panel, cx| {
            panel.delete_thread(editing.entity_id(), window, cx);
            panel.review_pending(cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert!(panel.composer.edit.is_none());
        assert!(
            !panel
                .threads
                .iter()
                .any(|thread| thread.entity_id() == editing.entity_id())
        );
        assert_eq!(panel.current().unwrap().entity_id(), owner.entity_id());
        assert!(!editing.read(cx).queue_editing);
    });
}

#[gpui_kit::test]
fn restart_during_edit_preserves_original_draft_scope_and_saved_model(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    old.update(cx, |thread, cx| {
        thread.last_model = Some("remembered model".into());
        thread.send("active".into(), cx);
        thread.attachments = vec![Attachment::Group("queued".into())];
        thread.send("queued prompt".into(), cx);
        thread.attachments = vec![Attachment::Group("default".into())];
    });
    cx.run_until_parked();
    set_input(&f, "original draft", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("queue-edit-first", cx);
        old.update(cx, |thread, cx| thread.fail("restart required", cx));
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        let current = panel.current().unwrap();
        assert_ne!(current.entity_id(), old.entity_id());
        assert_eq!(panel.input.read(cx).value().as_ref(), "original draft");
        assert!(panel.composer.edit.is_none());
        assert!(
            !panel
                .threads
                .iter()
                .any(|thread| thread.entity_id() == old.entity_id())
        );
        assert_eq!(
            current.read(cx).attachments,
            vec![Attachment::Group("default".into())]
        );
        assert_eq!(
            current.read(cx).model().as_deref(),
            Some("remembered model")
        );
        assert_eq!(current.read(cx).queue[0].saved.text, "queued prompt");
    });
}

#[gpui_kit::test]
fn editing_future_context_keeps_saved_defaults_and_active_grants(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.attachments.clear();
        thread.send("active without terminal".into(), cx);
        thread.attachments = vec![Attachment::Terminal(f.terminal)];
        thread.send("future with terminal".into(), cx);
        thread.attachments = vec![Attachment::Group("saved defaults".into())];
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
    })
    .unwrap();
    thread.update(cx, |thread, cx| {
        assert!(
            thread.resolved(cx).is_empty(),
            "future queue context granted tools to active turn"
        );
        assert_eq!(
            thread.snapshot(cx).unwrap().attachments,
            vec![SavedAttachment::Group("saved defaults".into())]
        );
    });
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| {
        assert!(
            thread.resolved(cx).is_empty(),
            "idle edit must still use original defaults"
        );
    });
}

#[gpui_kit::test]
fn mixed_unavailable_local_and_remote_queue_survives_save_and_edit(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let refs = vec![
        SavedAttachment::LocalTerminal("old shell".into()),
        SavedAttachment::Connection("deleted remote".into()),
    ];
    let mut saved = SavedChat::new("codex".into());
    saved.attachments = refs.clone();
    saved.queue = vec![SavedPrompt {
        id: 1,
        text: "retained".into(),
        images: Vec::new(),
        attachments: refs.clone(),
    }];
    let thread =
        cx.new(|cx| crate::thread::AgentThread::restored(saved, f.workspace.downgrade(), cx));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.track(&thread, window, cx);
            panel.threads.push(thread.clone());
            panel.open_thread(thread.entity_id(), cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        assert!(thread.resolved(cx).is_empty());
        assert_eq!(thread.snapshot(cx).unwrap().queue[0].attachments, refs);
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let labels = super::super::attachments::AttachmentLabels::new(&f.workspace.downgrade(), cx);
        assert!(
            labels
                .label(&thread.read(cx).queue[0].attachments[0])
                .contains("unavailable")
        );
        assert!(
            labels
                .label(&thread.read(cx).queue[0].attachments[1])
                .to_lowercase()
                .contains("unavailable")
        );
        window.click("queue-edit-first", cx);
        f.panel.update(cx, |panel, cx| {
            panel
                .input
                .update(cx, |input, cx| input.set_value("edited", window, cx))
        });
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        assert_eq!(thread.queue[0].saved.text, "edited");
        assert_eq!(thread.snapshot(cx).unwrap().queue[0].attachments, refs);
    });
    let disk = nocterm_ai::history::load_all(&f._directory.path().join("chats"));
    assert_eq!(disk[0].queue[0].attachments, refs);
}

#[gpui_kit::test]
fn unknown_slash_and_posix_paths_deliver_terminal_and_fallback_context(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    for input in [
        "/var/log/syslog investigate",
        "/unknown ordinary",
        "  /$skill:review аргументы",
    ] {
        thread.update(cx, |thread, cx| {
            thread.state.entries = vec![Entry::Agent("remember this context".into())];
            thread.fallback_history = true;
            thread.state.commands = vec![acp::AvailableCommand::new("$skill:review", "Review")];
            thread.send(input.into(), cx);
        });
        cx.run_until_parked();
        let prompts = f.commands.prompts.lock().unwrap();
        let request = prompts.last().unwrap();
        if input.contains("$skill:review") {
            assert_eq!(request.prompt.len(), 1);
            assert!(
                matches!(&request.prompt[0], acp::ContentBlock::Text(text) if text.text == input)
            );
        } else {
            assert_eq!(request.prompt.len(), 3);
            assert!(
                matches!(&request.prompt[0], acp::ContentBlock::Text(text) if text.text.contains(nocterm_ai::context::TERMINAL_RULES))
            );
            assert!(
                matches!(&request.prompt[1], acp::ContentBlock::Text(text) if text.text.contains("Assistant: remember this context"))
            );
        }
        drop(prompts);
        complete_active(&f, cx);
    }
}

#[gpui_kit::test]
fn copy_completed_message_uses_full_text_while_reveal_is_pending(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("question".into(), cx));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let full = "Ответ 👨‍👩‍👧‍👦 世界 ".repeat(1000);
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new(full.clone()),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    complete_active(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(f.panel.read(cx).stream.pending());
        window.hover(("agent-entry", 1usize), cx);
        window.click(("copy-message", 1usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), full);
        assert!(f.panel.read(cx).stream.pending());
    })
    .unwrap();
}

#[gpui_kit::test]
fn shared_snapshots_reuse_queued_payload_and_cache_exact_size(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        thread.images = vec![nocterm_ai::images::PromptImage::validate(png.into_inner()).unwrap()];
        thread.send("queued\n\"\\界".into(), cx);
        let first = thread.shared_snapshot(cx).unwrap();
        let second = thread.shared_snapshot(cx).unwrap();
        assert!(Arc::ptr_eq(&first.queue[0], &second.queue[0]));
        assert!(Arc::ptr_eq(&first.queue[0], &thread.queue[0].saved));
        assert_eq!(
            thread.queue[0].encoded_len,
            nocterm_ai::history::prompt_size(&thread.queue[0].saved).unwrap()
        );
    });
}

#[gpui_kit::test]
fn dirty_expanded_thought_and_offscreen_tool_remeasure_without_reveal_ticks(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.state.entries = vec![
            Entry::Thought("short".into()),
            Entry::Tool(acp::ToolCall::new("call", "Tool")),
        ];
        thread.state.entries.extend((0..30).map(|i| {
            Entry::User(vec![acp::ContentBlock::Text(acp::TextContent::new(
                format!("question {i}"),
            ))])
        }));
        for index in 0..thread.state.entries.len() {
            thread.mark_dirty(index);
        }
        cx.notify();
    });
    f.panel.update(cx, |panel, cx| {
        panel.expanded.extend([0, 1]);
        cx.notify();
    });
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            f.panel
                .read(cx)
                .list
                .clone()
                .update(cx, |list, cx| list.scroll_to_item(0, cx));
            window.render_frame(cx);
            window.find(("agent-entry", 0usize)).bounds().size.height
        })
        .unwrap();
    thread.update(cx, |thread, cx| {
        thread.state.entries[0] = Entry::Thought("thought line\n\n".repeat(10));
        thread.mark_dirty(0);
        cx.notify();
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thought = window.find(("agent-entry", 0usize)).bounds();
        assert!(thought.size.height > before);
        assert!(window.find(("agent-entry", 1usize)).bounds().origin.y >= thought.bottom());
        assert!(!f.panel.read(cx).stream.pending());
        f.panel
            .read(cx)
            .list
            .clone()
            .update(cx, |list, cx| list.scroll_to_item(30, cx));
        window.render_frame(cx);
        assert!(window.try_find(("tool-output", 1usize)).is_none());
    })
    .unwrap();
    thread.update(cx, |thread, cx| {
        thread.state.apply(acp::SessionUpdate::ToolCallUpdate(serde_json::from_value(serde_json::json!({
            "toolCallId":"call", "content":[{"type":"content", "content":{"type":"text", "text":"tool line\n".repeat(12)}}]
        })).unwrap()));
        thread.mark_dirty(1);
        cx.notify();
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        f.panel
            .read(cx)
            .list
            .clone()
            .update(cx, |list, cx| list.scroll_to_item(1, cx));
        window.render_frame(cx);
        let tool = window.find(("agent-entry", 1usize)).bounds();
        assert!(tool.size.height > gpui_kit::px(160.));
        assert!(window.find(("agent-entry", 2usize)).bounds().origin.y >= tool.bottom());
        assert!(!f.panel.read(cx).stream.pending());
    })
    .unwrap();
}

#[gpui_kit::test]
fn late_image_preparation_cannot_cross_chat_or_overwrite_its_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "saved chat", "saved answer", cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (send, receive) = oneshot::channel();
    old.update(cx, |thread, _| {
        thread
            .info
            .as_mut()
            .unwrap()
            .capabilities
            .prompt_capabilities
            .image = true
    });
    set_input(&f, "image owner draft", cx);
    f.panel.update(cx, |panel, cx| {
        panel.prepare_images(
            async move {
                receive.await.unwrap();
                let mut png = std::io::Cursor::new(Vec::new());
                image::RgbaImage::new(2, 2)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .unwrap();
                nocterm_ai::images::PromptImage::validate(png.into_inner()).map(|image| vec![image])
            },
            cx,
        )
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(f.panel.read(cx).preparing_images()));
    new_chat(&f, cx);
    set_input(&f, "second chat draft", cx);
    send.send(()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert!(!panel.preparing_images());
        assert_eq!(panel.input.read(cx).value().as_ref(), "second chat draft");
        assert!(panel.current().unwrap().read(cx).images.is_empty());
        assert!(old.read(cx).images.is_empty());
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(old.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "image owner draft"
        )
    });
}

#[gpui_kit::test]
fn advertised_command_removal_preserves_raw_unicode_caret_and_arguments(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.state.commands = vec![acp::AvailableCommand::new("обзор", "Review")];
        thread.state.commands_revision += 1;
        cx.notify();
    });
    set_input(&f, "/обзор args", cx);
    f.panel.update(cx, |panel, cx| {
        panel.input.update(cx, |input, cx| {
            input.set_selected_range("/обзор ar".len().."/обзор ar".len(), cx)
        })
    });
    cx.run_until_parked();
    let selection = cx.update(|cx| {
        let input = f.panel.read(cx).input.read(cx);
        assert_eq!(input.tokens().len(), 1);
        input.selected_range()
    });
    thread.update(cx, |thread, cx| {
        thread
            .state
            .apply(acp::SessionUpdate::AvailableCommandsUpdate(
                serde_json::from_value(serde_json::json!({"availableCommands":[]})).unwrap(),
            ));
        cx.notify();
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let input = f.panel.read(cx).input.read(cx);
        assert_eq!(input.value().as_ref(), "/обзор args");
        assert_eq!(input.selected_range(), selection);
        assert!(input.tokens().is_empty());
        window.input("界", cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/обзор ar界gs"
        )
    });
}

#[gpui_kit::test]
fn deleting_owner_cancels_pending_queue_image_edit_before_async_completion(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    owner.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        thread.images = vec![nocterm_ai::images::PromptImage::validate(png.into_inner()).unwrap()];
        thread.send("queued image".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
        assert!(f.panel.read(cx).preparing_queue_edit());
        f.panel.update(cx, |panel, cx| {
            panel.delete_thread(owner.entity_id(), window, cx);
            panel.new_thread("codex".into(), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert!(panel.composer.edit.is_none());
        assert!(!panel.preparing_images());
        assert_eq!(panel.input.read(cx).value().as_ref(), "");
        assert!(panel.current().unwrap().read(cx).images.is_empty());
        assert!(!owner.read(cx).queue_editing);
    });
}

#[gpui_kit::test]
async fn ai_off_clears_edit_drafts_stream_and_queue_measurements(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    owner.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("queued".into(), cx);
    });
    cx.run_until_parked();
    set_input(&f, "draft", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("queue-edit-first", cx);
        f.panel.update(cx, |panel, _| {
            panel
                .queue_heights
                .insert(owner.entity_id(), gpui_kit::px(99.));
        });
    })
    .unwrap();
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = f.panel.read(cx);
        assert!(panel.threads.is_empty());
        assert!(panel.composer.edit.is_none());

        assert!(panel.queue_heights.is_empty());
        assert!(!panel.stream.pending());
        assert!(panel.stream_tick.is_none());
        assert!(panel.input.read(cx).value().is_empty());
        assert!(!owner.read(cx).queue_editing);
    })
    .unwrap();
}

#[gpui_kit::test]
fn thousand_skill_menu_virtualizes_and_wraps_to_visible_variable_height_row(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.panel.update(cx, |panel, cx| panel.current().unwrap().update(cx, |thread, cx| {
        thread.state.commands = (0..1000).map(|index| {
            let mut value = serde_json::json!({"name":format!("skill{index:04}"), "description":"description ".repeat(1000)});
            if index % 2 == 1 { value["input"] = serde_json::json!({"hint":"Provide optional arguments"}); }
            serde_json::from_value(value).unwrap()
        }).collect();
        thread.state.commands_revision += 1;
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
        assert_eq!(f.panel.read(cx).commands.selected, 999);
        let row = window.find(("slash-command", 999usize)).bounds();
        let viewport = f.panel.read(cx).commands.scroll.viewport_bounds();
        assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom());
        assert_eq!(f.panel.read(cx).input.read(cx).selected_range(), 1..1);
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.press("tab", cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/skill0999 "
        );
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/skill0000 "
        );
        assert!(window.try_find(("slash-command", 0usize)).is_some());
    })
    .unwrap();
}
