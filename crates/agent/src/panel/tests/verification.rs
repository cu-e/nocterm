use super::*;

#[gpui_kit::test]
fn restart_retains_queue_with_unique_message_ids(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    old.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("retained first".into(), cx);
        thread.send("retained second".into(), cx);
    });
    cx.run_until_parked();
    old.update(cx, |thread, cx| thread.fail("Agent exited", cx));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-restart", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                assert_eq!(thread.queue.len(), 2);
                let retained = thread.queue[0].saved.id;
                thread.send_now(retained, cx);
                thread.send("new first".into(), cx);
                thread.send("new second".into(), cx);
                thread.send("new third".into(), cx);
                let ids = thread
                    .queue
                    .iter()
                    .map(|prompt| prompt.saved.id)
                    .collect::<Vec<_>>();
                let unique = ids
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>();
                assert_eq!(
                    ids.len(),
                    unique.len(),
                    "queued message ids collided: {ids:?}"
                );
            });
    });
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
        f.panel.update(cx, |panel, cx| {
            panel.input.update(cx, |input, cx| {
                input.set_value("edited after completion", window, cx)
            });
        });
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
fn slash_escape_and_tab_work_through_real_keyboard_events(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.current().unwrap().update(cx, |thread, cx| {
                thread.state.commands = vec![
                    acp::AvailableCommand::new("review", "Review changes"),
                    acp::AvailableCommand::new("rename", "Rename chat"),
                ];
                cx.notify();
            });
            panel
                .input
                .update(cx, |input, cx| input.set_value("/", window, cx));
        });
        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_some());
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "escape");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-slash-commands").is_none());
        assert_eq!(f.panel.read(cx).input.read(cx).value().as_ref(), "/");
        f.panel.update(cx, |panel, cx| {
            panel
                .input
                .update(cx, |input, cx| input.set_value("/r", window, cx));
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "tab");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(f.panel.read(cx).input.read(cx).value().as_ref(), "/review ");
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.render_frame(cx);
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
    cx.simulate_keystrokes(f.handle, "shift-tab");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(f.panel.read(cx).input.read(cx).value().as_ref(), "/rename ");
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_keystrokes(f.handle, "tab enter");
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(f.panel.read(cx).input.read(cx).value().as_ref(), "/review ");
        assert!(
            f.panel
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.input("these changes", cx);
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "/review these changes"
        );
    })
    .unwrap();
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn prompt_transport_failure_keeps_remaining_queue_and_rejects_new_dispatch(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("retained".into(), cx);
    });
    cx.run_until_parked();
    drop(f.commands.pending.lock().unwrap().take().unwrap());
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        assert!(thread.ended());
        assert!(thread.queue_paused);
        assert_eq!(thread.queue[0].saved.text, "retained");
        thread.send_now(thread.queue[0].saved.id, cx);
        assert!(!thread.submit("another".into(), None, cx).unwrap());
    });
    cx.run_until_parked();
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    let saved = nocterm_ai::history::load_all(&f._directory.path().join("chats"));
    assert_eq!(saved[0].queue[0].text, "retained");
}

#[gpui_kit::test]
fn streaming_keeps_reader_scroll_until_jump_to_latest(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    for index in 0..15 {
        exchange(
            &f,
            &format!("question {index}"),
            &"Long answer with wrapped text. ".repeat(30),
            cx,
        );
    }
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        f.panel.read(cx).list.clone().update(cx, |list, cx| {
            list.scroll_to_item(0, cx);
        });
        window.render_frame(cx);
        assert!(!f.panel.read(cx).list.read(cx).is_following_tail());
        assert!(f.panel.read(cx).list.read(cx).is_scrolled_up());
    })
    .unwrap();
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("new question".into(), cx));
    cx.run_until_parked();
    let session = cx.update(|cx| thread.read(cx).session.clone().unwrap());
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("new streamed answer".repeat(100)),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!f.panel.read(cx).list.read(cx).is_following_tail());
        assert!(f.panel.read(cx).list.read(cx).is_scrolled_up());
        window.click(
            (
                gpui_kit::ElementId::from("agent-transcript"),
                "jump-to-latest",
            ),
            cx,
        );
        window.render_frame(cx);
        assert!(f.panel.read(cx).list.read(cx).is_following_tail());
        assert!(!f.panel.read(cx).list.read(cx).is_scrolled_up());
    })
    .unwrap();
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn cancelling_queue_edit_does_not_resume_after_storage_failure(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("retained".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
    })
    .unwrap();
    let chats = f._directory.path().join("chats");
    std::fs::remove_dir_all(&chats).unwrap();
    std::fs::write(&chats, "blocks directory creation").unwrap();
    complete_active(&f, cx);
    cx.update(|cx| {
        assert!(thread.read(cx).queue_paused);
        assert!(thread.read(cx).status_error);
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
    cx.update(|cx| assert_eq!(thread.read(cx).queue.len(), 1));
}

#[gpui_kit::test]
fn cancelling_queue_edit_does_not_resume_after_explicit_stop(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
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
    cx.update(|cx| assert_eq!(thread.read(cx).queue.len(), 1));
}

fn disk_chat(f: &Fixture, id: &str) -> nocterm_ai::history::SavedChat {
    nocterm_ai::history::load_all(&f._directory.path().join("chats"))
        .into_iter()
        .find(|chat| chat.id == id)
        .unwrap()
}

fn reopen(
    f: &Fixture,
    saved: nocterm_ai::history::SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    let thread =
        cx.new(|cx| crate::thread::AgentThread::restored(saved, f.workspace.downgrade(), cx));
    f.panel.update(cx, |panel, cx| panel.wake(&thread, cx));
    cx.run_until_parked();
    thread
}

#[gpui_kit::test]
fn unsent_history_survives_reopen_and_slash_only_then_clears_on_disk(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "remember the first disk", "the first disk is full", cx);
    let saved = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .snapshot(cx)
            .unwrap()
    });
    let id = saved.id.clone();
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::RestoreUnavailable(
            "Original session is gone".into(),
        ));
    let first = reopen(&f, saved, cx);
    let fresh = disk_chat(&f, &id);
    assert!(fresh.pending_history);
    assert_ne!(fresh.session_id.as_deref(), Some("s0"));
    drop(first);
    cx.run_until_parked();

    // Native resume succeeds for the new session, which has not received old messages.
    let second = reopen(&f, fresh, cx);
    cx.update(|cx| assert!(second.read(cx).fallback_history));
    second.update(cx, |thread, cx| {
        thread.state.commands = vec![acp::AvailableCommand::new("help", "Show help")];
        thread.send("/help".into(), cx);
    });
    cx.run_until_parked();
    complete_active(&f, cx);
    let after_slash = disk_chat(&f, &id);
    assert!(after_slash.pending_history);
    drop(second);
    cx.run_until_parked();

    let third = reopen(&f, after_slash, cx);
    third.update(cx, |thread, cx| thread.send("what was full?".into(), cx));
    cx.run_until_parked();
    {
        let prompts = f.commands.prompts.lock().unwrap();
        let request = prompts.last().unwrap();
        let history = request
            .prompt
            .iter()
            .filter_map(|block| match block {
                acp::ContentBlock::Text(text) if text.text.contains("Saved conversation") => {
                    Some(&text.text)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(history.len(), 1);
        assert!(history[0].contains("User: remember the first disk"));
        assert!(history[0].contains("Assistant: the first disk is full"));
    }
    assert!(
        disk_chat(&f, &id).pending_history,
        "delivery has not finished"
    );
    complete_active(&f, cx);
    let delivered = disk_chat(&f, &id);
    assert!(!delivered.pending_history);
    drop(third);
    cx.run_until_parked();

    let fourth = reopen(&f, delivered, cx);
    cx.update(|cx| assert!(!fourth.read(cx).fallback_history));
    fourth.update(cx, |thread, cx| thread.send("next question".into(), cx));
    cx.run_until_parked();
    assert!(!f.commands.prompts.lock().unwrap().last().unwrap().prompt.iter().any(
        |block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("Saved conversation"))
    ));
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn partial_fork_keeps_pending_context_until_an_ordinary_prompt(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "original question", "original answer", cx);
    exchange(&f, "later question", "later answer", cx);
    let source = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread_at(source.entity_id(), Some(1), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let fork = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = cx.update(|cx| fork.read(cx).chat_id.clone());
    assert!(disk_chat(&f, &id).pending_history);
    assert!(
        f.commands.restores.lock().unwrap().is_empty(),
        "partial fork needs a fresh session"
    );
    fork.update(cx, |thread, cx| {
        thread.state.commands = vec![acp::AvailableCommand::new("help", "Show help")];
        thread.send("/help".into(), cx);
    });
    cx.run_until_parked();
    complete_active(&f, cx);
    let restarted = reopen(&f, disk_chat(&f, &id), cx);
    restarted.update(cx, |thread, cx| {
        thread.send("continue the original".into(), cx)
    });
    cx.run_until_parked();
    let prompts = f.commands.prompts.lock().unwrap();
    let history = prompts
        .last()
        .unwrap()
        .prompt
        .iter()
        .find_map(|block| match block {
            acp::ContentBlock::Text(text) if text.text.contains("Saved conversation") => {
                Some(&text.text)
            }
            _ => None,
        })
        .unwrap();
    assert!(history.contains("original question") && history.contains("original answer"));
    assert!(!history.contains("later question") && !history.contains("later answer"));
    drop(prompts);
    complete_active(&f, cx);
    assert!(!disk_chat(&f, &id).pending_history);
}

#[gpui_kit::test]
fn cancelled_fallback_delivery_keeps_history_across_restart(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = cx.update(|cx| thread.read(cx).chat_id.clone());
    thread.update(cx, |thread, cx| {
        thread.state.entries = vec![nocterm_ai::thread::Entry::Agent("old answer".into())];
        thread.fallback_history = true;
        thread.send("interrupted question".into(), cx);
        thread.stop(cx);
    });
    cx.run_until_parked();
    f.commands
        .pending
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(acp::PromptResponse::new(acp::StopReason::Cancelled))
        .unwrap();
    cx.run_until_parked();
    assert!(disk_chat(&f, &id).pending_history);
    thread.update(cx, |thread, cx| thread.fail("restart needed", cx));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let restarted = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update(|cx| assert!(restarted.read(cx).fallback_history));
    restarted.update(cx, |thread, cx| thread.send("try again".into(), cx));
    cx.run_until_parked();
    assert!(f.commands.prompts.lock().unwrap().last().unwrap().prompt.iter().any(
        |block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("Saved conversation") && text.text.contains("old answer"))
    ));
    complete_active(&f, cx);
    assert!(!disk_chat(&f, &id).pending_history);
}
