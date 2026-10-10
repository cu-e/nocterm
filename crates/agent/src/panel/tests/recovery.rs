//! A chat goes on after its session fails: forks, resumes and transport
//! errors recover through the next message instead of a reload.
use super::*;
use std::time::Duration;

fn overloaded() -> AgentError {
    AgentError::Rpc(acp::Error::internal_error().data("Service temporarily overloaded"))
}

fn last_prompt_text(f: &Fixture) -> Vec<String> {
    f.commands
        .prompts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .prompt
        .iter()
        .filter_map(|block| match block {
            acp::ContentBlock::Text(text) => Some(text.text.clone()),
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
        assert!(!fork.ended(), "{}", fork.status);
        assert!(fork.lifecycle.generating());
    });
    let text = last_prompt_text(&f);
    let history = text
        .iter()
        .find(|text| text.contains("Saved conversation"))
        .expect("the new session receives the forked conversation");
    assert!(history.contains("restart nginx") && history.contains("Use systemctl."));
    assert_eq!(text.last().unwrap(), "and reload it");
    complete_active(&f, cx);
    cx.update(|cx| assert!(!fork.read(cx).fallback_history));
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
        assert!(thread.read(cx).lifecycle.generating());
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
    cx.update(|cx| assert!(thread.read(cx).lifecycle.generating()));
}
