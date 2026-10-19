//! Editing a queued message pauses dispatch and keeps every other draft intact.
use super::drafts::{disk_value, type_draft};
use super::*;
use nocterm_ai::history::{SavedAttachment, SavedChat, SavedPrompt};
use std::time::Duration;

fn pending_permission(
    f: &Fixture,
    thread: &Entity<crate::thread::AgentThread>,
    cx: &mut TestAppContext,
) {
    let (send, _) = nocterm_ai::PermissionResponder::channel();
    thread.update(cx, |thread, _| {
        let request = serde_json::from_value(serde_json::json!({
            "sessionId":thread.session().as_ref().unwrap(),
            "toolCall":{"toolCallId":"pending","title":"Needs approval"},
            "options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]
        }))
        .unwrap();
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
            editing.read(cx).composer().attachments(),
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
    let mut saved = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-4000-8000-000000000069".into(),
        1,
    );
    saved.attachments = refs.clone();
    saved.queue = vec![SavedPrompt {
        id: 1,
        text: "retained".into(),
        images: Vec::new(),
        attachments: refs.clone(),
    }];
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
        assert_eq!(thread.snapshot(cx).unwrap().queue[0].attachments, refs);
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let labels = super::super::attachments::AttachmentLabels::new(&f.workspace.downgrade(), cx);
        assert!(
            labels
                .label(&thread.read(cx).composer().queue()[0].attachments[0])
                .contains("unavailable")
        );
        assert!(
            labels
                .label(&thread.read(cx).composer().queue()[0].attachments[1])
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
        assert_eq!(thread.composer().queue()[0].saved.text, "edited");
        assert_eq!(thread.snapshot(cx).unwrap().queue[0].attachments, refs);
    });
    let disk = f.repository.chats();
    assert_eq!(disk[0].queue[0].attachments, refs);
}

#[gpui_kit::test]
fn edit_pauses_dispatch_until_save_after_active_response_finishes(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("queued".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
    })
    .unwrap();
    complete_active(&f, cx);
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(
            f.panel.read(cx).composer_focus(cx),
            "edited after completion",
            window,
            cx,
        );

        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let requests = f.commands.prompts.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        matches!(requests[1].prompt.last().unwrap(), acp::ContentBlock::Text(text) if text.text == "edited after completion")
    );
    drop(requests);
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn cancelling_queue_edit_does_not_resume_after_storage_failure(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("active".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.send("retained".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
    })
    .unwrap();
    let id = cx.update(|cx| thread.read(cx).document().chat_id().to_owned());
    f.repository.reject(
        nocterm_ai::testing::StorageOperation::Save(id),
        nocterm_ai::storage::AgentStorageError::Directory {
            path: f._directory.path().join("chats"),
            source: Arc::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "blocks directory creation",
            )),
        },
    );
    thread.update(cx, |thread, cx| {
        report_control(
            thread,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("durable response before completion"),
            ))),
            cx,
        );
    });
    complete_active(&f, cx);
    cx.update(|cx| {
        assert!(thread.read(cx).composer().paused());
        assert!(thread.read(cx).has_status_error());
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-cancel-edit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        f.commands.prompts.lock().unwrap().len(),
        1,
        "cancelling edit resumed a queue paused by storage failure"
    );
    cx.update(|cx| assert_eq!(thread.read(cx).composer().queue().len(), 1));
}

#[gpui_kit::test]
fn cancelling_queue_edit_does_not_resume_after_explicit_stop(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("active".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.send("retained".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
        window.dispatch_action(Box::new(crate::StopGeneration), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.cancels.load(Ordering::SeqCst), 1);
    complete_active(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-cancel-edit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        f.commands.prompts.lock().unwrap().len(),
        1,
        "cancelling edit resumed a queue explicitly stopped by the user"
    );
    cx.update(|cx| assert_eq!(thread.read(cx).composer().queue().len(), 1));
}

#[gpui_kit::test]
fn queued_message_edit_keeps_the_original_draft_on_disk(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("queued original".into(), cx);
    });
    cx.run_until_parked();
    type_draft(&f, "separate unsent draft", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
    })
    .unwrap();
    type_draft(&f, "replacement being edited", cx);
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let id = thread.read_with(cx, |thread, _| thread.document().chat_id().to_owned());
    let before = disk_value(&f.repository, &id);
    assert_eq!(before["draft"], "separate unsent draft");
    assert_eq!(before["queue"][0]["text"], "queued original");
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let after = disk_value(&f.repository, &id);
    assert_eq!(after["draft"], "separate unsent draft");
    assert_eq!(after["queue"][0]["text"], "replacement being edited");
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "separate unsent draft"
        );
    });
}

#[gpui_kit::test]
fn a_restored_text_prompt_is_edited_with_its_own_context(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let mut saved = super::restoration::empty_chat(&f);
    saved.attachments = vec![SavedAttachment::Group("defaults".into())];
    saved.queue.push(SavedPrompt {
        id: 1,
        text: "queued text".into(),
        images: Vec::new(),
        attachments: vec![SavedAttachment::Group("prompt context".into())],
    });
    let thread = super::restoration::adopt_saved(&mut f, saved, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.edit_queued(1, window, cx))
    })
    .unwrap();
    thread.update(cx, |thread, _| {
        assert!(thread.composer().editing());
        assert_eq!(
            thread.composer().attachments().to_vec(),
            [Attachment::Group("prompt context".into())]
        );
    });
    type_draft(&f, "edited text", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, _| {
        assert!(!thread.composer().editing());
        assert_eq!(thread.composer().queue()[0].saved.text, "edited text");
        assert_eq!(
            thread.composer().attachments().to_vec(),
            [Attachment::Group("defaults".into())]
        );
    });
}
