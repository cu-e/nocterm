//! Saved JSON enters the same lazy activation and queue path as real history.
use super::*;
use nocterm_ai::history::{SavedChat, SavedPrompt};
use std::time::Duration;

fn adopt_json(
    f: &Fixture,
    chat: SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    cx.run_until_parked();
    let dir = f._directory.path().join("chats");
    nocterm_ai::history::save(&dir, &chat).unwrap();
    let saved = nocterm_ai::history::load_all(&dir);
    assert_eq!(saved.len(), 1);
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, _| runtime.saved_chats = Some(saved)));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.adopt_saved_chats(window, cx);
            let thread = panel.threads[0].clone();
            panel.open_thread(thread.entity_id(), cx);
            thread
        })
    })
    .unwrap()
}

fn saved_chat(f: &Fixture, history: bool) -> SavedChat {
    let mut chat = SavedChat::new("codex".into());
    chat.session_id = Some("unpersisted-rollout".into());
    chat.workdir = Some(f._directory.path().join("work"));
    chat.draft = Some("new submission".into());
    chat.queue = vec![SavedPrompt {
        id: 1,
        text: "previous queued message".into(),
        images: Vec::new(),
        attachments: Vec::new(),
    }];
    if history {
        chat.entries = vec![nocterm_ai::thread::Entry::User(vec![
            acp::ContentBlock::Text(acp::TextContent::new("prior real message")),
        ])];
    }
    chat
}

fn send_composer(f: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.send(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn persisted_empty_history_with_a_stale_id_starts_fresh_and_keeps_queued_fifo(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(cx);
    let thread = adopt_json(&f, saved_chat(&f, false), cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    send_composer(&f, cx);
    assert!(
        f.commands.restores.lock().unwrap().is_empty(),
        "empty transcript tried to restore a provider rollout"
    );
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(thread.read(cx).lifecycle.generating());
        assert!(!thread.read(cx).status_error);
        assert_eq!(
            thread.read(cx).composer.queue[0].saved.text,
            "new submission"
        );
    });
    let text = |index: usize| {
        f.commands.prompts.lock().unwrap()[index]
            .prompt
            .iter()
            .filter_map(|block| match block {
                acp::ContentBlock::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .next_back()
            .unwrap()
    };
    assert_eq!(text(0), "previous queued message");
    complete_active(&f, cx);
    assert_eq!(text(1), "new submission");
    complete_active(&f, cx);
    cx.update(|cx| assert!(thread.read(cx).composer.queue.is_empty()));
}

#[gpui_kit::test]
fn a_nonmissing_rpc_failure_does_not_replace_real_history_or_dispatch_its_queue(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = adopt_json(&f, saved_chat(&f, true), cx);
    for _ in 0..2 {
        f.commands
            .restore_errors
            .lock()
            .unwrap()
            .push_back(AgentError::Rpc(
                acp::Error::internal_error().data("Service temporarily overloaded"),
            ));
    }
    send_composer(&f, cx);
    cx.background_executor
        .advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(f.commands.restores.lock().unwrap().len(), 2);
    assert_eq!(
        f.commands.sessions.load(Ordering::SeqCst),
        0,
        "generic RPC failure silently discarded real provider state"
    );
    assert!(f.commands.prompts.lock().unwrap().is_empty());
    cx.update(|cx| {
        assert!(thread.read(cx).status_error);
        assert!(thread.read(cx).composer.queue_paused);
        assert_eq!(thread.read(cx).composer.queue.len(), 2);
        assert_eq!(thread.read(cx).state.entries.len(), 1);
    });
}
