use super::*;
use nocterm_ai::history::{SavedAttachment, SavedChat, SavedPrompt};
use nocterm_ai::{
    model::{
        ids::ToolId,
        notice::AdvertisedCommand,
        tools::{ToolChanges, ToolPatch, ToolStatus},
    },
    thread::Entry,
};

pub(super) fn set_input(f: &Fixture, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(f.panel.read(cx).composer_focus(cx), text, window, cx);

        window.render_frame(cx);
    })
    .unwrap();
}
fn pending_permission(
    f: &Fixture,
    thread: &Entity<crate::thread::AgentThread>,
    cx: &mut TestAppContext,
) {
    let (send, _) = nocterm_ai::PermissionResponder::channel();
    thread.update(cx, |thread, _| {
        let request = super::provider_permissions::allow_once(
            thread.session().as_ref().unwrap().clone(),
            "pending",
            "Needs approval",
        );
        f.events
            .try_send(AgentEvent::Permission {
                request,
                respond: send,
            })
            .unwrap();
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn approval_switch_restores_each_draft_and_queue_edit_defaults(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    set_input(&f, "owner draft", cx);
    pending_permission(&f, &owner, cx);
    new_chat(&f, cx);
    let editing = cx.update(|cx| f.panel.read(cx).current().unwrap());
    editing.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.replace_composer_context(vec![Attachment::Group("queued scope".into())], cx);
        thread.send("queued".into(), cx);
        thread.replace_composer_context(vec![Attachment::Group("draft scope".into())], cx);
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
        assert_eq!(panel.composer_text(cx).as_ref(), "owner draft");
        assert!(
            panel
                .current()
                .is_none_or(|thread| !thread.read(cx).composer().editing_queue())
        );
        assert!(!editing.read(cx).composer().editing_queue());
        assert_eq!(
            editing.read(cx).composer().attachments().to_vec(),
            vec![Attachment::Group("draft scope".into())]
        );
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(editing.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
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
    pending_permission(&f, &owner, cx);
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
        assert!(
            panel
                .current()
                .is_none_or(|thread| !thread.read(cx).composer().editing_queue())
        );
        assert!(
            !panel
                .chat_documents()
                .any(|thread| thread.entity_id() == editing.entity_id())
        );
        assert_eq!(panel.current().unwrap().entity_id(), owner.entity_id());
        assert!(!editing.read(cx).composer().editing_queue());
    });
}

#[gpui_kit::test]
fn restart_during_edit_preserves_original_draft_scope_and_saved_model(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    old.update(cx, |thread, cx| {
        let mut option = super::composer::select(
            "model",
            "Model",
            "saved",
            vec![("saved", "remembered model")],
        );
        option.category = Some(nocterm_ai::model::controls::ConfigCategory::Model);
        report_control(&f, thread, SessionUpdate::Configuration(vec![option]), cx);
        thread.send("active".into(), cx);
        thread.replace_composer_context(vec![Attachment::Group("queued".into())], cx);
        thread.send("queued prompt".into(), cx);
        thread.replace_composer_context(vec![Attachment::Group("default".into())], cx);
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
        assert_eq!(panel.composer_text(cx).as_ref(), "original draft");
        assert!(
            panel
                .current()
                .is_none_or(|thread| !thread.read(cx).composer().editing_queue())
        );
        assert!(
            !panel
                .chat_documents()
                .any(|thread| thread.entity_id() == old.entity_id())
        );
        assert_eq!(
            current.read(cx).composer().attachments().to_vec(),
            vec![Attachment::Group("default".into())]
        );
        assert_eq!(
            current.read(cx).model().as_deref(),
            Some("remembered model")
        );
        assert_eq!(
            current.read(cx).composer().queued_rows()[0].text(),
            "queued prompt"
        );
    });
}

#[gpui_kit::test]
fn editing_future_context_keeps_saved_defaults_and_active_grants(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.clear_composer_context(cx);
        thread.send("active without terminal".into(), cx);
        thread.replace_composer_context(vec![Attachment::Terminal(f.terminal)], cx);
        thread.send("future with terminal".into(), cx);
        thread.replace_composer_context(vec![Attachment::Group("saved defaults".into())], cx);
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
            thread.snapshot(cx).unwrap().composer().attachments,
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
    let saved = SavedChat::for_identity(
        &f.installation.retention,
        "codex".into(),
        "00000000-0000-4000-8000-000000000069".into(),
        1,
    );
    let (id, metadata, resume, transcript, mut composer, _) = saved.into_parts();
    composer.attachments = refs.clone();
    composer.queue = vec![SavedPrompt {
        id: 1,
        text: "retained".into(),
        images: Vec::new(),
        attachments: refs.clone(),
    }];
    let saved = SavedChat::from_parts(
        &f.installation.retention,
        id,
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    let thread = super::archive::archived(&f, &saved, cx);
    show_history(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("history-thread", thread.entity_id().as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        assert!(thread.resolved(cx).is_empty());
        assert_eq!(
            thread.snapshot(cx).unwrap().composer().queue[0].attachments,
            refs
        );
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let labels = super::super::attachments::AttachmentLabels::new(&f.workspace.downgrade(), cx);
        assert!(
            labels
                .label(&thread.read(cx).composer().queued_rows()[0].attachments()[0])
                .contains("unavailable")
        );
        assert!(
            labels
                .label(&thread.read(cx).composer().queued_rows()[0].attachments()[1])
                .to_lowercase()
                .contains("unavailable")
        );
        window.click("queue-edit-first", cx);
        replace_composer_value(f.panel.read(cx).composer_focus(cx), "edited", window, cx);

        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        assert_eq!(thread.composer().queued_rows()[0].text(), "edited");
        assert_eq!(
            thread.snapshot(cx).unwrap().composer().queue[0].attachments,
            refs
        );
    });
    let disk = f.repository.chats();
    assert_eq!(disk[0].composer().queue[0].attachments, refs);
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
            restore_entries(
                &f,
                thread,
                vec![Entry::Agent("remember this context".into())],
                cx,
            );
            restore_pending_history(&f, thread, cx);
            report_control(
                &f,
                thread,
                SessionUpdate::Commands(vec![AdvertisedCommand::new("$skill:review", "Review")]),
                cx,
            );
            thread.send(input.into(), cx);
        });
        cx.run_until_parked();
        let prompts = f.commands.prompts.lock().unwrap();
        let request = prompts.last().unwrap();
        if input.contains("$skill:review") {
            assert_eq!(request.content.len(), 1);
            assert!(matches!(&request.content[0], Content::Text(text) if text.text == input));
        } else {
            assert_eq!(request.content.len(), 3);
            assert!(
                matches!(&request.content[0], Content::Text(text) if text.text.contains(nocterm_ai::context::TERMINAL_RULES))
            );
            assert!(
                matches!(&request.content[1], Content::Text(text) if text.text.contains("Assistant: remember this context"))
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
        .try_send(AgentEvent::Session(SessionNotice {
            session,
            update: SessionUpdate::AgentChunk(Content::Text(Text {
                text: full.clone(),
                annotations: None,
                extensions: None,
            })),
        }))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    complete_active(&f, cx);
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            let height = window.find(("agent-entry", 1usize)).bounds().size.height;
            window.hover(("agent-entry", 1usize), cx);
            window.click(("copy-message", 1usize), cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), full);
            height
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find(("agent-entry", 1usize)).bounds().size.height > before,
            "visible streamed text must continue revealing after copy"
        );
        window.hover(("agent-entry", 1usize), cx);
        window.click(("copy-message", 1usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), full);
    })
    .unwrap();
}

#[gpui_kit::test]
fn shared_snapshots_reuse_queued_payload_and_cache_exact_size(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("active".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        attach_image(thread, image_bytes(), image_codec(), cx);
        thread.send("queued\n\"\\界".into(), cx);
        let saved = thread.snapshot(cx).unwrap();
        let queued = &saved.composer().queue[0];
        assert_eq!(queued.text, "queued\n\"\\界");
        assert_eq!(queued.images.len(), 1);
        assert_eq!(thread.composer().queue()[0].images().len(), 1);
        assert_eq!(thread.composer().queue()[0].text().to_string(), queued.text);
    });
}

/// A chat whose first row is an expanded thought that is still streaming.
fn expanded_thought(f: &Fixture, cx: &mut TestAppContext) -> Entity<crate::thread::AgentThread> {
    new_chat(f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    deliver(f, &stream_session(&thread, cx), thought_chunk("short"), cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    thread
}
fn stream_session(
    thread: &Entity<crate::thread::AgentThread>,
    cx: &mut TestAppContext,
) -> SessionId {
    cx.update(|cx| thread.read(cx).session().clone().unwrap())
}
fn thought_chunk(text: &str) -> SessionUpdate {
    SessionUpdate::ThoughtChunk(Content::Text(Text {
        text: text.into(),
        annotations: None,
        extensions: None,
    }))
}
/// Adds a pending tool and thirty answered questions after the thought, then expands the tool.
fn add_tool_and_questions(
    f: &Fixture,
    thread: &Entity<crate::thread::AgentThread>,
    cx: &mut TestAppContext,
) {
    let session = stream_session(thread, cx);
    deliver(
        f,
        &session,
        SessionUpdate::ToolStarted(super::provider_intersections::provider_tool(
            "call",
            "Tool",
            ToolStatus::Pending,
        )),
        cx,
    );
    for i in 0..30 {
        thread.update(cx, |thread, cx| thread.send(format!("question {i}"), cx));
        cx.run_until_parked();
        complete_active(f, cx);
    }
    thread.update(cx, |thread, cx| {
        for index in 0..thread.history_rows().len() {
            thread.mark_dirty(index);
        }
        cx.notify();
    });
    super::navigation::scroll_to(f, 0, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 1usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn assert_static_row(f: &Fixture, index: usize, cx: &mut TestAppContext) {
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(("agent-entry", index)).bounds()
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(("agent-entry", index)).bounds(), before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn dirty_expanded_thought_and_offscreen_tool_remeasure_without_reveal_ticks(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = expanded_thought(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("thought", 0usize), cx)
    })
    .unwrap();
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(("agent-entry", 0usize)).bounds().size.height
        })
        .unwrap();
    let session = stream_session(&thread, cx);
    deliver(
        &f,
        &session,
        thought_chunk(&"thought line\n\n".repeat(10)),
        cx,
    );
    add_tool_and_questions(&f, &thread, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thought = window.find(("agent-entry", 0usize)).bounds();
        assert!(thought.size.height > before);
        assert!(window.find(("agent-entry", 1usize)).bounds().origin.y >= thought.bottom());
    })
    .unwrap();
    assert_static_row(&f, 0, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(
            (
                gpui_kit::ElementId::from("agent-transcript"),
                "jump-to-latest",
            ),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, _| {
        assert!(window.try_find(("tool-output", 1usize)).is_none());
    })
    .unwrap();
    thread.update(cx, |thread, cx| {
        report_control(
            &f,
            thread,
            SessionUpdate::ToolChanged(ToolPatch {
                id: ToolId::new("call"),
                changes: ToolChanges {
                    output: Some(vec![super::tool_output::text_output(
                        "tool line\n".repeat(12),
                    )]),
                    ..Default::default()
                },
                extensions: None,
                claims: Default::default(),
                literal: Default::default(),
            }),
            cx,
        );
        thread.mark_dirty(1);
        cx.notify();
    });
    super::navigation::scroll_to(&f, 1, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let tool = window.find(("agent-entry", 1usize)).bounds();
        assert!(tool.size.height > gpui_kit::px(160.));
        assert!(window.find(("agent-entry", 2usize)).bounds().origin.y >= tool.bottom());
    })
    .unwrap();
    assert_static_row(&f, 1, cx);
}

#[gpui_kit::test]
fn late_image_preparation_cannot_cross_chat_or_overwrite_its_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.connector.support_images();
    new_chat(&f, cx);
    exchange(&f, "saved chat", "saved answer", cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (send, receive) = oneshot::channel();
    set_input(&f, "image owner draft", cx);
    f.panel.update(cx, |panel, cx| {
        panel.prepare_images(
            async move {
                receive.await.unwrap();
                Ok(vec![image_bytes()])
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
        assert_eq!(panel.composer_text(cx).as_ref(), "second chat draft");
        assert!(
            panel
                .current()
                .unwrap()
                .read(cx)
                .composer()
                .images()
                .is_empty()
        );
        assert!(old.read(cx).composer().images().is_empty());
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(old.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
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
        report_control(
            &f,
            thread,
            SessionUpdate::Commands(vec![AdvertisedCommand::new("обзор", "Review")]),
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
        report_control(&f, thread, SessionUpdate::Commands(Vec::new()), cx);
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
fn deleting_owner_ends_a_queue_image_edit_without_leaking_it_into_the_next_chat(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    owner.update(cx, |thread, cx| thread.send("active".into(), cx));
    cx.run_until_parked();
    owner.update(cx, |thread, cx| {
        attach_image(thread, image_bytes(), image_codec(), cx);
        thread.send("queued image".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
        assert!(owner.read(cx).composer().editing_queue());
        f.panel.update(cx, |panel, cx| {
            panel.delete_thread(owner.entity_id(), window, cx);
            panel.new_thread("codex".into(), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert!(
            panel
                .current()
                .is_none_or(|thread| !thread.read(cx).composer().editing_queue())
        );
        assert!(!panel.preparing_images());
        assert_eq!(panel.composer_text(cx).as_ref(), "");
        assert!(
            panel
                .current()
                .unwrap()
                .read(cx)
                .composer()
                .images()
                .is_empty()
        );
        assert!(!owner.read(cx).composer().editing_queue());
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
        window.render_frame(cx);
        assert!(window.find("agent-queue").bounds().size.height > gpui_kit::px(0.));
    })
    .unwrap();
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = f.panel.read(cx);
        assert!(panel.chat_documents().len() == 0);
        assert!(
            panel
                .current()
                .is_none_or(|thread| !thread.read(cx).composer().editing_queue())
        );

        assert!(window.try_find("agent-queue").is_none());
        assert!(window.try_find("agent-transcript-area").is_none());
        assert!(panel.composer_text(cx).is_empty());
        assert!(!owner.read(cx).composer().editing_queue());
    })
    .unwrap();
}
