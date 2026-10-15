use super::*;

fn texts(f: &Fixture) -> Vec<String> {
    f.commands
        .prompts
        .lock()
        .unwrap()
        .iter()
        .map(|request| {
            request
                .prompt
                .iter()
                .rev()
                .find_map(|block| match block {
                    acp::ContentBlock::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
                .unwrap()
        })
        .collect()
}
#[gpui_kit::test]
fn queued_prompts_are_fifo_edit_in_place_and_snapshot_context(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.replace_composer_context(vec![Attachment::Group("second context".into())], cx);
        assert!(thread.submit("second".into(), None, cx).unwrap());
        thread.replace_composer_context(vec![Attachment::Group("third context".into())], cx);
        assert!(thread.submit("third".into(), None, cx).unwrap());
        let id = thread.composer().queue()[0].saved.id;
        thread.replace_composer_context(vec![Attachment::Group("edited context".into())], cx);
        assert!(thread.submit("edited second".into(), Some(id), cx).unwrap());
        assert_eq!(thread.composer().queue().len(), 2);
        assert_eq!(thread.composer().queue()[0].saved.id, id);
        assert_eq!(thread.composer().queue()[1].saved.text, "third");
        let error = thread
            .submit("edited again".into(), Some(u64::MAX), cx)
            .unwrap_err();
        assert_eq!(error.to_string(), "This message was already sent.");
    });
    cx.run_until_parked();
    assert_eq!(texts(&f), ["first"]);
    let saved = f.repository.chats();
    assert_eq!(saved[0].queue.len(), 2);
    assert_eq!(saved[0].queue[0].text, "edited second");
    complete_active(&f, cx);
    assert_eq!(texts(&f), ["first", "edited second"]);
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).prompt_scope().as_ref().unwrap(),
            &[Attachment::Group("edited context".into())]
        )
    });
    complete_active(&f, cx);
    assert_eq!(texts(&f), ["first", "edited second", "third"]);
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).prompt_scope().as_ref().unwrap(),
            &[Attachment::Group("third context".into())]
        )
    });
    complete_active(&f, cx);
    cx.update(|cx| assert!(thread.read(cx).composer().queue().is_empty()));
}
#[gpui_kit::test]
fn send_now_waits_for_cancellation_then_retains_other_messages(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.send("second".into(), cx);
        thread.send("priority".into(), cx);
        let priority = thread.composer().queue()[1].saved.id;
        thread.send_now(priority, cx);
    });
    cx.run_until_parked();
    assert_eq!(f.commands.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(texts(&f), ["first"]);
    complete_active(&f, cx);
    assert_eq!(texts(&f), ["first", "priority"]);
    // The cancelled turn's watchdog must not terminate this new turn.
    cx.background_executor
        .advance_clock(std::time::Duration::from_secs(11));
    cx.run_until_parked();
    cx.update(|cx| assert!(!thread.read(cx).ended()));
    complete_active(&f, cx);
    assert_eq!(texts(&f), ["first", "priority", "second"]);
    complete_active(&f, cx);
}
#[gpui_kit::test]
fn stopping_or_auth_failure_retains_queue_without_auto_resend(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.send("second".into(), cx);
        thread.stop(cx);
    });
    cx.run_until_parked();
    complete_active(&f, cx);
    assert_eq!(texts(&f), ["first"]);
    cx.update(|cx| assert_eq!(thread.read(cx).composer().queue().len(), 1));
    f.commands.auth_required.store(true, Ordering::SeqCst);
    thread.update(cx, |thread, cx| {
        let id = thread.composer().queue()[0].saved.id;
        thread.send_now(id, cx);
        thread.send("third".into(), cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(thread.read(cx).session_sign_in_required());
        assert_eq!(thread.read(cx).composer().queue()[0].saved.text, "third");
    });
    assert_eq!(texts(&f), ["first", "second"]);
}
#[gpui_kit::test]
fn queue_edit_restores_the_unsent_composer_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                thread.send("active".into(), cx);
                thread.send("queued".into(), cx);
            });
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(
            f.panel.read(cx).composer_focus(cx),
            "unsent draft",
            window,
            cx,
        );

        window.render_frame(cx);
        window.click("queue-edit-first", cx);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "queued");
        replace_composer_value(
            f.panel.read(cx).composer_focus(cx),
            "replacement",
            window,
            cx,
        );
        f.panel.update(cx, |panel, cx| {
            panel.send(window, cx);
        });
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "unsent draft");
        assert_eq!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .composer()
                .queue()[0]
                .saved
                .text,
            "replacement"
        );
    })
    .unwrap();
    complete_active(&f, cx);
    assert_eq!(texts(&f), ["active", "replacement"]);
    complete_active(&f, cx);
}
#[gpui_kit::test]
fn slash_popup_keyboard_completion_keeps_commands_as_raw_prompts(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                report_control(
                    thread,
                    acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
                        vec![
                            acp::AvailableCommand::new("plan", "Plan a task"),
                            acp::AvailableCommand::new("review", "Review changes"),
                        ],
                    )),
                    cx,
                );
                cx.notify();
            });
    });
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(f.panel.read(cx).composer_focus(cx), "/", window, cx);

        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_some());
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "down enter");
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "/review ");
        assert!(f.commands.prompts.lock().unwrap().is_empty());
        f.panel.update(cx, |panel, cx| panel.send(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(texts(&f), ["/review "]);
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn restoration_keeps_descriptor_through_auth_and_falls_back_only_if_unavailable(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "remember disk", "disk is full", cx);
    let saved = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .snapshot(cx)
            .unwrap()
    });
    f.panel.update(cx, |panel, cx| {
        panel
            .current()
            .unwrap()
            .update(cx, |thread, cx| thread.release_resources(cx));
    });
    cx.run_until_parked();
    let thread = cx.new(|cx| {
        crate::thread::AgentThread::restored(
            saved.clone(),
            f._directory.host(f.workspace.downgrade()),
            cx,
        )
    });
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::AuthRequired("Sign in".into()));
    cx.update(|cx| activate_thread(&thread, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(thread.read(cx).session_sign_in_required());
        assert_eq!(
            thread.read(cx).snapshot(cx).unwrap().session_id,
            saved.session_id
        );
        let restore = thread.read(cx).pending_restore().as_ref().unwrap();
        assert_eq!(
            restore.session.0.as_ref(),
            saved.session_id.as_deref().unwrap()
        );
    });
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::RestoreUnavailable("No native resume".into()));
    cx.update_window(f.handle, |_, window, cx| {
        thread.update(cx, |thread, cx| {
            thread.authenticate("sign-in".into(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        assert!(thread.document().pending_history());
        thread.send("what now?".into(), cx);
    });
    cx.run_until_parked();
    let requests = f.commands.prompts.lock().unwrap();
    let prompt = &requests.last().unwrap().prompt;
    assert!(prompt.iter().any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("User: remember disk") && text.text.contains("Assistant: disk is full"))));
    drop(requests);
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| {
        assert!(!thread.document().pending_history());
        thread.send("next".into(), cx);
    });
    cx.run_until_parked();
    let requests = f.commands.prompts.lock().unwrap();
    assert!(!requests.last().unwrap().prompt.iter().any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("Saved conversation"))));
    drop(requests);
    complete_active(&f, cx);
}

#[test]
fn slash_prefixes_do_not_capture_normal_text_or_command_arguments() {
    let commands = [acp::AvailableCommand::new("plan", "Plan")];
    assert_eq!(crate::panel::commands::matches("/PL", &commands).len(), 1);
    for text in [
        "path/to/file",
        "/plan arguments",
        "/plan\n",
        "ordinary message",
    ] {
        assert!(crate::panel::commands::matches(text, &commands).is_empty());
    }
}

#[gpui_kit::test]
fn controls_received_before_session_response_are_kept(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (ready, gate) = oneshot::channel();
    *f.commands.session_gate.lock().unwrap() = Some(gate);
    new_chat(&f, cx);
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            "s0",
            acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(vec![
                acp::AvailableCommand::new("early", "Early command"),
            ])),
        )))
        .unwrap();
    cx.run_until_parked();
    ready.send(()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .configuration()
                .commands()[0]
                .name,
            "early"
        )
    });
}

#[gpui_kit::test]
fn transient_restore_retries_without_starting_an_empty_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "remember", "yes", cx);
    let saved = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .snapshot(cx)
            .unwrap()
    });
    f.panel.update(cx, |panel, cx| {
        panel
            .current()
            .unwrap()
            .update(cx, |thread, cx| thread.release_resources(cx));
    });
    cx.run_until_parked();
    let thread = cx.new(|cx| {
        crate::thread::AgentThread::restored(
            saved.clone(),
            f._directory.host(f.workspace.downgrade()),
            cx,
        )
    });
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::Io("temporary".into()));
    let count = f.commands.sessions.load(Ordering::SeqCst);
    cx.update(|cx| activate_thread(&thread, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).snapshot(cx).unwrap().session_id,
            saved.session_id
        )
    });
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), count);
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).session().as_ref().unwrap().0.as_ref(),
            saved.session_id.as_deref().unwrap()
        );
        assert!(!thread.read(cx).document().pending_history());
    });
}

#[gpui_kit::test]
fn failed_persistence_pauses_but_keeps_the_queue(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = cx.update(|cx| thread.read(cx).document().chat_id().to_owned());
    thread.update(cx, |thread, cx| thread.send("active".into(), cx));
    cx.run_until_parked();
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
        thread.send("retained".into(), cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(thread.read(cx).composer().paused());
        assert!(thread.read(cx).has_status_error());
        assert!(thread.read(cx).status().contains("Could not save chat"));
        assert_eq!(thread.read(cx).composer().queue()[0].saved.text, "retained");
    });
    complete_active(&f, cx);
    assert_eq!(texts(&f), ["active"]);
}

#[gpui_kit::test]
fn switching_chat_during_queue_edit_preserves_each_composer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    old.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("queued".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(f.panel.read(cx).composer_focus(cx), "old draft", window, cx);

        window.render_frame(cx);
        window.click("queue-edit-first", cx);
        f.panel
            .update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            f.panel
                .read(cx)
                .current()
                .is_none_or(|thread| !thread.read(cx).composer().editing_queue())
        );
        assert!(f.panel.read(cx).composer_text(cx).is_empty());
        assert_eq!(old.read(cx).composer().queue()[0].saved.text, "queued");
        assert_eq!(
            old.read(cx).composer().attachments(),
            [Attachment::Terminal(f.terminal)]
        );
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(old.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "old draft"));
    complete_active(&f, cx);
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn shutdown_flush_preserves_queue_and_stable_context(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    *f.access.profile().borrow_mut() = Some("saved-server".into());
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.add_composer_attachment(Attachment::Group("production".into()), cx);
        thread.send("after restart".into(), cx);
    });
    cx.run_until_parked();
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx).detach()));
    let saved = f.repository.chats();
    assert_eq!(saved[0].queue[0].text, "after restart");
    assert!(
        saved[0]
            .attachments
            .contains(&nocterm_ai::history::SavedAttachment::Connection(
                "saved-server".into()
            ))
    );
    assert!(
        saved[0]
            .attachments
            .contains(&nocterm_ai::history::SavedAttachment::Group(
                "production".into()
            ))
    );
    assert_eq!(saved[0].queue[0].attachments, saved[0].attachments);
}

#[gpui_kit::test]
fn slash_commands_match_adapter_first_block_and_preserve_history_fallback(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        restore_entries(
            &f,
            thread,
            vec![nocterm_ai::thread::Entry::Agent("saved context".into())],
            cx,
        );
        restore_pending_history(&f, thread, cx);
        report_control(
            thread,
            acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(vec![
                acp::AvailableCommand::new("usage", "Usage"),
                acp::AvailableCommand::new("review", "Review"),
            ])),
            cx,
        );
        thread.send("/usage".into(), cx);
    });
    cx.run_until_parked();
    {
        let requests = f.commands.prompts.lock().unwrap();
        let blocks = &requests.last().unwrap().prompt;
        assert_eq!(
            blocks.len(),
            1,
            "administrative commands require one text block"
        );
        assert!(matches!(&blocks[0], acp::ContentBlock::Text(text) if text.text == "/usage"));
    }
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| {
        assert!(
            thread.document().pending_history(),
            "local ACP commands did not rebuild the model's context"
        );
        thread.send("/review these changes".into(), cx);
    });
    cx.run_until_parked();
    {
        let requests = f.commands.prompts.lock().unwrap();
        assert!(
            matches!(&requests.last().unwrap().prompt[0], acp::ContentBlock::Text(text) if text.text == "/review these changes")
        );
    }
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| thread.send("ordinary question".into(), cx));
    cx.run_until_parked();
    {
        let requests = f.commands.prompts.lock().unwrap();
        assert!(requests.last().unwrap().prompt.iter().any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("Saved conversation") && text.text.contains("saved context"))));
    }
    complete_active(&f, cx);
    cx.update(|cx| assert!(!thread.read(cx).document().pending_history()));
}

#[gpui_kit::test]
fn pending_image_preparation_blocks_early_submit_and_retains_prompt(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.connector.support_images();
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (ready, wait) = oneshot::channel();
    let bytes = image_bytes();
    let codec = image_codec();
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(
            f.panel.read(cx).composer_focus(cx),
            "with image",
            window,
            cx,
        );
        f.panel.update(cx, |panel, cx| {
            panel.prepare_images(
                async move {
                    wait.await.unwrap();
                    Ok(vec![nocterm_ai::images::PromptImage::validate(
                        bytes,
                        codec.as_ref(),
                    )?])
                },
                cx,
            );
            panel.send(window, cx);
            assert!(panel.preparing_images());
            assert_eq!(panel.composer_text(cx).as_ref(), "with image");
        });
    })
    .unwrap();
    cx.update_window(f.handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
    ready.send(()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!f.panel.read(cx).preparing_images());
        assert_eq!(thread.read(cx).composer().images().len(), 1);
    });
    cx.update_window(f.handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    let requests = f.commands.prompts.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .prompt
            .iter()
            .any(|block| matches!(block, acp::ContentBlock::Image(_)))
    );
    assert!(
        requests[0].prompt.iter().any(
            |block| matches!(block, acp::ContentBlock::Text(text) if text.text == "with image")
        ),
        "{:?}",
        requests[0].prompt
    );
    drop(requests);
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn late_image_preparation_cannot_attach_to_a_different_chat(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.connector.support_images();
    new_chat(&f, cx);
    let (ready, wait) = oneshot::channel();
    f.panel.update(cx, |panel, cx| {
        panel.prepare_images(
            async move {
                wait.await.unwrap();
                Err(super::super::images::ImageAttachError::Read(
                    std::io::Error::other("late invalid image"),
                ))
            },
            cx,
        )
    });
    new_chat(&f, cx);
    ready.send(()).unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-error").is_none());
    })
    .unwrap();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert!(!panel.preparing_images());
        assert!(
            panel
                .current()
                .unwrap()
                .read(cx)
                .composer()
                .images()
                .is_empty()
        );
    });
}
