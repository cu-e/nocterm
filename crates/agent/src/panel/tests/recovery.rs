//! A chat goes on after its session fails: forks, resumes and transport
//! errors recover through the next message instead of a reload.
use super::*;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::controls::{ConfigCategory, ConfigChoice, ConfigOption};
use nocterm_ai::model::session::SessionUpdate;
use std::time::Duration;

fn overloaded() -> AgentError {
    AgentError::Rpc(nocterm_ai::model::session::RemoteError {
        code: -32603,
        message: "Service temporarily overloaded".into(),
    })
}

fn last_prompt_text(f: &Fixture) -> Vec<String> {
    f.commands
        .prompts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .content
        .iter()
        .filter_map(|block| match block {
            Content::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

fn send(thread: &Entity<crate::thread::AgentThread>, text: &str, cx: &mut TestAppContext) {
    thread.update(cx, |thread, cx| thread.send(text.into(), cx));
    cx.background_executor
        .advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_fork_the_agent_cannot_copy_starts_fresh_and_sends(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let source_session = exchange(&f, "restart nginx", "Use systemctl.", cx);
    let source = cx.update(|cx| f.panel.read(cx).current().unwrap());
    // The fork connects as soon as it is shown, so it meets these first.
    for _ in 0..2 {
        f.commands
            .restore_errors
            .lock()
            .unwrap()
            .push_back(overloaded());
    }
    let sessions = f.commands.sessions.load(Ordering::SeqCst);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread(source.entity_id(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let fork = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_ne!(fork.entity_id(), source.entity_id());

    send(&fork, "and reload it", cx);

    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(source_session.clone(), true), (source_session, true)]
    );
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), sessions + 1);
    cx.update(|cx| {
        let fork = fork.read(cx);
        assert!(!fork.ended(), "{}", fork.status());
        assert!(fork.session_generating());
    });
    let text = last_prompt_text(&f);
    let history = text
        .iter()
        .find(|text| text.contains("Saved conversation"))
        .expect("the new session receives the forked conversation");
    assert!(history.contains("restart nginx") && history.contains("Use systemctl."));
    assert_eq!(text.last().unwrap(), "and reload it");
    complete_active(&f, cx);
    cx.update(|cx| assert!(!fork.read(cx).document().pending_history()));
}

#[gpui_kit::test]
fn a_session_that_keeps_failing_to_resume_is_replaced(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let session = exchange(&f, "check the disk", "Use df -h.", cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    // The chat goes idle; its session must be reopened.
    thread.update(cx, |thread, cx| thread.detach_session(cx));
    cx.run_until_parked();
    let sessions = f.commands.sessions.load(Ordering::SeqCst);

    // A transient error keeps the session for the next try.
    for _ in 0..2 {
        f.commands
            .restore_errors
            .lock()
            .unwrap()
            .push_back(overloaded());
    }
    send(&thread, "first try", cx);
    assert_eq!(f.commands.restores.lock().unwrap().len(), 2);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), sessions);
    cx.update(|cx| assert!(thread.read(cx).ended()));

    // The same session is tried once more.
    for _ in 0..2 {
        f.commands
            .restore_errors
            .lock()
            .unwrap()
            .push_back(overloaded());
    }
    send(&thread, "second try", cx);
    assert_eq!(f.commands.restores.lock().unwrap().len(), 4);
    assert!(
        f.commands
            .restores
            .lock()
            .unwrap()
            .iter()
            .all(|(id, fork)| *id == session && !fork)
    );
    cx.update(|cx| assert!(thread.read(cx).ended()));

    // Then the chat stops insisting and starts a new session with its
    // conversation.
    send(&thread, "third try", cx);
    assert_eq!(f.commands.restores.lock().unwrap().len(), 4);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), sessions + 1);
    cx.update(|cx| {
        assert!(!thread.read(cx).ended());
        assert!(thread.read(cx).session_generating());
    });
    let text = last_prompt_text(&f);
    assert!(text.iter().any(|text| text.contains("check the disk")));
    assert_eq!(text.last().unwrap(), "first try");
}

#[gpui_kit::test]
fn after_an_agent_crash_one_failed_resume_is_enough(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "check the disk", "Use df -h.", cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.fail("Agent exited", cx));
    cx.run_until_parked();
    for _ in 0..2 {
        f.commands
            .restore_errors
            .lock()
            .unwrap()
            .push_back(overloaded());
    }
    send(&thread, "first try", cx);
    cx.update(|cx| assert!(thread.read(cx).ended()));
    send(&thread, "second try", cx);
    assert_eq!(f.commands.restores.lock().unwrap().len(), 2);
    cx.update(|cx| assert!(thread.read(cx).session_generating()));
}

#[gpui_kit::test]
fn release_changes_notice_text_and_activation_clears_its_error_emphasis(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.fail("Agent unavailable", cx);
        assert!(thread.has_status_error());
        thread.release_resources(cx);
        assert_eq!(
            thread.status(),
            "Agent resources released. Conversation retained."
        );
        assert!(
            thread.has_status_error(),
            "a text-only notice retains the failure emphasis"
        );
        thread.request_activation(cx);
        assert_eq!(thread.status(), "Waiting for an agent slot…");
        assert!(!thread.has_status_error());
        thread.set_shown(false, cx);
        assert!(thread.status().is_empty());
        assert!(!thread.is_leased());
    });
}

#[gpui_kit::test]
fn restart_during_edit_preserves_original_draft_scope_and_saved_model(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    old.update(cx, |thread, cx| {
        let option = ConfigOption::select(
            "model",
            "Model",
            "saved",
            vec![ConfigChoice::new("saved", "remembered model")],
        )
        .category(ConfigCategory::Model);
        report_control(thread, SessionUpdate::ConfigOptionUpdate(vec![option]), cx);
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
            current.read(cx).composer().attachments(),
            vec![Attachment::Group("default".into())]
        );
        assert_eq!(
            current.read(cx).model().as_deref(),
            Some("remembered model")
        );
        assert_eq!(
            current.read(cx).composer().queue()[0].saved.text,
            "queued prompt"
        );
    });
}

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
                assert_eq!(thread.composer().queue().len(), 2);
                let retained = thread.composer().queue()[0].saved.id;
                thread.send_now(retained, cx);
                thread.send("new first".into(), cx);
                thread.send("new second".into(), cx);
                thread.send("new third".into(), cx);
                let ids = thread
                    .composer()
                    .queue()
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
fn prompt_transport_failure_keeps_the_queue_until_the_user_reconnects(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("active".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.send("retained".into(), cx);
    });
    cx.run_until_parked();
    drop(f.commands.pending.lock().unwrap().take().unwrap());
    cx.run_until_parked();
    thread.update(cx, |thread, _| {
        assert!(thread.ended());
        assert!(thread.composer().paused());
        assert_eq!(thread.composer().queue()[0].saved.text, "retained");
    });
    // Nothing reconnects by itself.
    cx.run_until_parked();
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    let saved = f.repository.chats();
    assert_eq!(saved[0].queue[0].text, "retained");

    // Sending reconnects the chat and goes on with its queue.
    thread.update(cx, |thread, cx| {
        thread.send_now(thread.composer().queue()[0].saved.id, cx);
        assert!(thread.submit("another".into(), None, cx).unwrap());
    });
    cx.run_until_parked();
    thread.update(cx, |thread, _| {
        assert!(!thread.ended());
        assert!(thread.session_generating());
        assert_eq!(thread.composer().queue().len(), 1);
        assert_eq!(thread.composer().queue()[0].saved.text, "another");
    });
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].content.iter().any(|block| matches!(
        block,
        Content::Text(text) if text.text == "retained"
    )));
}
