//! Empty provider sessions have no conversation to restore.
use super::*;
use nocterm_ai::{
    history::{SavedChat, SavedPrompt},
    thread::Entry,
};

fn adopt_saved(
    f: &Fixture,
    chat: SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| runtime.saved_chats = Some(vec![chat]))
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| f.panel.read(cx).threads[0].clone());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    thread
}

fn empty_chat(f: &Fixture) -> SavedChat {
    let mut chat = SavedChat::new("codex".into());
    chat.session_id = Some("never-persisted-provider-session".into());
    chat.workdir = Some(f._directory.path().join("work"));
    chat.draft = Some("literal unsent text".into());
    chat
}

#[gpui_kit::test]
fn draft_only_history_starts_one_fresh_session_without_restoring(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = adopt_saved(&f, empty_chat(&f), cx);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 0);
    assert!(f.commands.restores.lock().unwrap().is_empty());
    cx.update(|cx| {
        let snapshot = thread.read(cx).snapshot(cx).unwrap();
        assert!(snapshot.session_id.is_none());
        assert!(snapshot.workdir.is_none());
        assert_eq!(snapshot.draft.as_deref(), Some("literal unsent text"));
    });
    thread.update(cx, |thread, cx| {
        thread.send("first actual request".into(), cx)
    });
    cx.run_until_parked();
    assert!(f.commands.restores.lock().unwrap().is_empty());
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    cx.update(|cx| assert!(!thread.read(cx).fallback_history));
}

#[gpui_kit::test]
fn empty_fork_descriptor_with_a_queued_prompt_does_not_restore(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut saved = empty_chat(&f);
    saved.queue.push(SavedPrompt {
        id: 1,
        text: "waiting request".into(),
        images: Vec::new(),
        attachments: Vec::new(),
    });
    let thread = adopt_saved(&f, saved, cx);
    thread.update(cx, |thread, cx| {
        thread.restore.as_mut().unwrap().fork = true;
        thread.send_now(1, cx);
    });
    cx.run_until_parked();
    assert!(f.commands.restores.lock().unwrap().is_empty());
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].prompt.iter().any(
        |block| matches!(block, acp::ContentBlock::Text(text) if text.text == "waiting request")
    ));
    cx.update(|cx| assert!(!thread.read(cx).fallback_history));
}

#[gpui_kit::test]
fn missing_rollout_falls_back_once_and_preserves_history_and_queue(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut saved = empty_chat(&f);
    saved.entries = vec![
        Entry::User(vec![acp::ContentBlock::Text(acp::TextContent::new(
            "remember this",
        ))]),
        Entry::Agent("previous answer".into()),
    ];
    saved.queue.push(SavedPrompt {
        id: 1,
        text: "continue conversation".into(),
        images: Vec::new(),
        attachments: Vec::new(),
    });
    let thread = adopt_saved(&f, saved, cx);
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::RestoreUnavailable(
            "no rollout found for thread id previous".into(),
        ));
    thread.update(cx, |thread, cx| thread.send_now(1, cx));
    cx.run_until_parked();
    assert_eq!(f.commands.restores.lock().unwrap().len(), 1);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].prompt.iter().any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("User: remember this") && text.text.contains("Assistant: previous answer"))));
    assert!(prompts[0].prompt.iter().any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text == "continue conversation")));
    drop(prompts);
    cx.update(|cx| assert_eq!(thread.read(cx).state.entries.len(), 3));
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| thread.send("next message".into(), cx));
    cx.run_until_parked();
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(!prompts[1].prompt.iter().any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("Saved conversation (context"))));
    assert_eq!(f.commands.restores.lock().unwrap().len(), 1);
}
