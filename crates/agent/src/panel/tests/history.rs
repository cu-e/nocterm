use super::*;

#[gpui_kit::test]
fn chats_are_saved_restored_into_a_new_panel_and_resume_their_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let chats = f._directory.path().join("chats");
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
        thread.read(cx).session().clone().unwrap()
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session.clone(),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("answer"),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish
        .send(acp::PromptResponse::new(acp::StopReason::EndTurn))
        .unwrap();
    cx.run_until_parked();

    let saved = nocterm_ai::history::load_all(&chats);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].session_id.as_deref(), Some(&*session.0));
    assert!(matches!(
        saved[0].entries.as_slice(),
        [nocterm_ai::thread::Entry::User(_), nocterm_ai::thread::Entry::Agent(answer)] if answer == "answer"
    ));
    // The terminal context sent with the prompt is not part of the chat.
    let text = std::fs::read_to_string(chats.join(format!("{}.json", saved[0].id))).unwrap();
    assert!(
        !text.contains(nocterm_ai::context::TERMINAL_RULES),
        "{text}"
    );

    // As after a restart: the previous process has closed before another document resumes it.
    f.panel.update(cx, |panel, cx| {
        panel
            .current()
            .unwrap()
            .update(cx, |thread, cx| thread.release_resources(cx));
    });
    cx.run_until_parked();
    // A new panel shows the saved chat, not yet connected.
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| runtime.saved_chats = Some(saved.clone()))
    });
    let panel = cx
        .update_window(f.handle, |_, window, cx| {
            cx.new(|cx| AgentPanel::new(f.workspace.downgrade(), window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| {
        let panel = panel.read(cx);
        assert_eq!(panel.threads.len(), 1);
        panel.threads[0].clone()
    });
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert!(thread.dormant);
        assert_eq!(thread.state.entries.len(), 2);
        assert!(thread.session().is_none());
    });
    assert!(f.commands.restores.lock().unwrap().is_empty());

    // Opening it reopens the agent's session instead of starting another.
    let sessions = f.commands.sessions.load(Ordering::SeqCst);
    panel.update(cx, |panel, cx| panel.wake(&thread, cx));
    cx.run_until_parked();
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(session.clone(), false)]
    );
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), sessions);
    cx.update(|cx| assert_eq!(thread.read(cx).session().as_ref(), Some(&session)));

    let id = cx.update(|cx| thread.read(cx).chat_id.clone());
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.delete_chat(id, cx)));
    cx.run_until_parked();
    assert!(nocterm_ai::history::load_all(&chats).is_empty());
}
/// Sends `text` in the open chat and lets the agent answer `answer`.
#[gpui_kit::test]
fn empty_chats_are_dropped_when_another_starts_and_restart_keeps_the_chat(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    new_chat(&f, cx);
    cx.update(|cx| assert_eq!(f.panel.read(cx).threads.len(), 1));
    exchange(&f, "check the disk", "Use df -h.", cx);
    new_chat(&f, cx);
    cx.update(|cx| assert_eq!(f.panel.read(cx).threads.len(), 2));

    // A chat whose agent exited is restarted in place, as the same chat.
    let (old, chat) = cx.update(|cx| {
        let panel = f.panel.read(cx);
        let old = panel.threads[0].clone();
        let chat = old.read(cx).chat_id.clone();
        (old, chat)
    });
    let id = old.entity_id();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.open_thread(id, cx);
            panel
                .current()
                .unwrap()
                .update(cx, |thread, cx| thread.fail("Agent exited", cx));
        });
        window.render_frame(cx);
        window.click("agent-restart", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        // The empty chat was dropped too: nothing was said in it.
        assert_eq!(panel.threads.len(), 1);
        let thread = panel.current().unwrap().read(cx);
        assert_ne!(panel.current().unwrap().entity_id(), id);
        assert_eq!(thread.chat_id, chat);
        assert_eq!(thread.title(), "check the disk");
        assert!(!thread.ended());
    });
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn history_searches_pins_renames_and_forks_chats(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let chats = f._directory.path().join("chats");
    new_chat(&f, cx);
    exchange(&f, "check disk usage", "Use df -h.", cx);
    let disk = cx.update(|cx| f.panel.read(cx).current().unwrap());
    new_chat(&f, cx);
    let nginx_session = exchange(&f, "restart nginx", "Done.", cx);
    let nginx = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let row = |thread: &Entity<crate::thread::AgentThread>| {
        ("history-thread", thread.entity_id().as_u64())
    };

    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.history = true;
            panel
                .history_search
                .update(cx, |search, cx| search.set_value("DF -H", window, cx));
        });
        window.render_frame(cx);
        // Messages are searched as well as titles, ignoring case.
        assert!(window.try_find(row(&disk)).is_some());
        assert!(window.try_find(row(&nginx)).is_none());
        f.panel.update(cx, |panel, cx| {
            panel.history_search.update(cx, |search, cx| {
                search.set_value("nothing like it", window, cx)
            })
        });
        window.render_frame(cx);
        assert!(window.try_find("agent-history-no-match").is_some());
        f.panel.update(cx, |panel, cx| {
            panel
                .history_search
                .update(cx, |search, cx| search.set_value("", window, cx))
        });
        window.render_frame(cx);
        // The newest chat comes first, until the older one is pinned.
        assert!(
            window.find(row(&nginx)).bounds().origin.y < window.find(row(&disk)).bounds().origin.y
        );
        f.panel
            .update(cx, |panel, cx| panel.set_pinned(disk.entity_id(), true, cx));
        window.render_frame(cx);
        assert!(
            window.find(row(&disk)).bounds().origin.y < window.find(row(&nginx)).bounds().origin.y
        );

        f.panel.update(cx, |panel, cx| {
            panel.start_rename(disk.entity_id(), window, cx)
        });
        window.render_frame(cx);
        f.panel.update(cx, |panel, cx| {
            let input = panel.renaming.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| input.set_value("  Disks  ", window, cx));
            panel.finish_rename(true, window, cx);
        });
        window.render_frame(cx);
        assert_eq!(window.find(row(&disk)).label(), Some("Disks"));
    })
    .unwrap();
    cx.run_until_parked();
    let saved = nocterm_ai::history::load_all(&chats);
    assert_eq!(saved[0].name.as_deref(), Some("Disks"));
    assert!(saved[0].pinned);

    // A fork starts with the chat's messages, in a copy of its session.
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread(nginx.entity_id(), window, cx);
            let fork = panel.current().unwrap();
            panel.wake(&fork, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert_eq!(panel.threads.len(), 3);
        assert!(panel.history, "history stays beside the chat");
        let fork = panel.current().unwrap().read(cx);
        assert_eq!(fork.title(), "restart nginx (fork)");
        assert_eq!(fork.state.entries.len(), 2);
        assert_eq!(
            fork.session().clone(),
            Some(acp::SessionId::new(format!("fork-of-{nginx_session}")))
        );
        assert_ne!(fork.chat_id, nginx.read(cx).chat_id);
    });
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(nginx_session, true)]
    );
    assert_eq!(nocterm_ai::history::load_all(&chats).len(), 3);
}

#[test]
fn history_relative_time_boundaries() {
    use std::time::Duration;
    assert_eq!(
        crate::panel::widgets::relative_prompt_time(None),
        "No requests yet"
    );
    for (seconds, expected) in [
        (0, "Just now"),
        (59, "Just now"),
        (60, "1 minute ago"),
        (120, "2 minutes ago"),
        (3599, "59 minutes ago"),
        (3600, "1 hour ago"),
        (7200, "2 hours ago"),
        (86400, "1 day ago"),
        (172800, "2 days ago"),
    ] {
        assert_eq!(
            crate::panel::widgets::relative_prompt_time(Some(Duration::from_secs(seconds))),
            expected
        );
    }
}

#[gpui_kit::test]
fn history_records_only_accepted_prompts_and_keeps_model_snapshot(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            assert!(thread.last_prompt.is_none());
            thread.send("  ".into(), cx);
            assert!(thread.last_prompt.is_none());
            let session = thread.lease.as_mut().unwrap().session.take();
            thread.send("Queued before the session is ready".into(), cx);
            assert_eq!(thread.composer.queue.len(), 1);
            thread.composer.queue.clear();
            assert!(thread.last_prompt.is_none());
            thread.lease.as_mut().unwrap().session = session;
            let mut option = acp::SessionConfigOption::select(
                "model",
                "Agent model",
                "first",
                vec![acp::SessionConfigSelectOption::new("first", "First model")],
            );
            option.category = Some(acp::SessionConfigOptionCategory::Model);
            thread.state.config_options = vec![option];
            thread.send("First request".into(), cx);
            let prompt = thread.last_prompt.clone().unwrap();
            assert_eq!(prompt.model.as_deref(), Some("First model"));
            assert_eq!(thread.state.entries.len(), 1);
            thread.state.config_options.clear();
            thread.send("Rejected while busy".into(), cx);
            assert_eq!(thread.state.entries.len(), 1);
            assert_eq!(thread.last_prompt.as_ref().unwrap().model, prompt.model);
            thread.state.title = Some("Updated title".into());
            thread.stop(cx);
            thread.send("Rejected while stopping".into(), cx);
            assert_eq!(thread.state.entries.len(), 1);
        });
    });
    cx.update_window(f.handle, |_, window, cx| {
        let id = f.panel.read(cx).current().unwrap().entity_id().as_u64();
        window.render_frame(cx);
        let row = window.find(("history-thread", id)).bounds();
        let delete = window.find(("delete-thread", id)).bounds();
        let history = window.find("agent-history-list").bounds();
        assert!(row.size.width > history.size.width * 0.85);
        // The whole row is one target, its buttons included.
        assert!(row.origin.x < delete.origin.x && delete.right() <= row.right());
        assert_eq!(
            window.find(("history-thread", id)).label(),
            Some("Updated title")
        );
        assert!(f.panel.read(cx).history_tick.is_some());
        window.click(("history-thread", id), cx);
        assert!(f.panel.read(cx).history, "history stays beside the chat");
        window.click("agent-history", cx);
        assert!(!f.panel.read(cx).history);
        window.render_frame(cx);
        assert!(f.panel.read(cx).history_tick.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn history_starts_open_beside_the_first_chat(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let id = f.panel.read(cx).current().unwrap().entity_id().as_u64();
        window.render_frame(cx);
        assert!(window.find(("history-thread", id)).visible());
        window.click("agent-history", cx);
        window.render_frame(cx);
        assert!(window.try_find(("history-thread", id)).is_none());
    })
    .unwrap();
}
