//! Empty provider sessions have no conversation to restore.
use super::*;
use nocterm_ai::{
    history::{SavedChat, SavedPrompt},
    thread::Entry,
};
use std::time::Duration;

/// Opens `chat` without warm start, so it restores when work starts.
pub(super) fn adopt_saved(
    f: &mut Fixture,
    chat: SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    lazy_start(f, cx);
    reinstall_history(f, vec![chat], cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| f.panel.read(cx).chat_documents().next().unwrap().clone());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    thread
}

pub(super) fn empty_chat(f: &Fixture) -> SavedChat {
    let mut chat = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-4000-8000-00000000006f".into(),
        1,
    );
    chat.session_id = Some("never-persisted-provider-session".into());
    chat.workdir = Some(f._directory.path().join("work"));
    chat.draft = Some("literal unsent text".into());
    chat
}

#[gpui_kit::test]
fn draft_only_history_starts_one_fresh_session_without_restoring(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let saved = empty_chat(&f);
    let thread = adopt_saved(&mut f, saved, cx);
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
    cx.update(|cx| assert!(!thread.read(cx).document().pending_history()));
}

#[gpui_kit::test]
fn empty_fork_descriptor_with_a_queued_prompt_does_not_restore(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let saved = empty_chat(&f);
    let source = adopt_saved(&mut f, saved, cx);
    let thread = cx
        .update_window(f.handle, |_, window, cx| {
            f.panel.update(cx, |panel, cx| {
                panel.fork_thread(source.entity_id(), window, cx)
            });
            f.panel.read(cx).current().unwrap()
        })
        .unwrap();
    thread.update(cx, |thread, cx| {
        assert!(
            thread
                .pending_restore()
                .as_ref()
                .is_some_and(|restore| restore.fork)
        );
        assert!(thread.submit("waiting request".into(), None, cx).unwrap());
        let id = thread.queued_prompt_ids().next().unwrap();
        thread.send_now(id, cx);
    });
    cx.run_until_parked();
    assert!(f.commands.restores.lock().unwrap().is_empty());
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].prompt.iter().any(
        |block| matches!(block, acp::ContentBlock::Text(text) if text.text == "waiting request")
    ));
    cx.update(|cx| assert!(!thread.read(cx).document().pending_history()));
}

#[gpui_kit::test]
fn missing_rollout_falls_back_once_and_preserves_history_and_queue(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
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
    let thread = adopt_saved(&mut f, saved, cx);
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
    cx.update(|cx| assert_eq!(thread.read(cx).transcript().entries.len(), 3));
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| thread.send("next message".into(), cx));
    cx.run_until_parked();
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(!prompts[1].prompt.iter().any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("Saved conversation (context"))));
    assert_eq!(f.commands.restores.lock().unwrap().len(), 1);
}

pub(super) fn disk_chat(f: &Fixture, id: &str) -> nocterm_ai::history::SavedChat {
    f.repository
        .chats()
        .into_iter()
        .find(|chat| chat.id == id)
        .unwrap()
}

pub(super) fn reopen(
    f: &Fixture,
    saved: nocterm_ai::history::SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    f.panel.update(cx, |panel, cx| {
        for thread in panel.chat_documents() {
            if thread.read(cx).document().chat_id() == saved.id {
                thread.update(cx, |thread, cx| thread.release_resources(cx));
            }
        }
    });
    cx.run_until_parked();
    let thread = cx.new(|cx| {
        crate::thread::AgentThread::restored(
            saved,
            f._directory
                .host(f.workspace.downgrade(), f.installation.installed.clone()),
            cx,
        )
    });
    cx.update(|cx| activate_thread(&thread, &f.installation.installed, cx));
    cx.run_until_parked();
    thread
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
    let id = cx.update(|cx| fork.read(cx).document().chat_id().to_owned());
    assert!(disk_chat(&f, &id).pending_history);
    assert!(
        f.commands.restores.lock().unwrap().is_empty(),
        "partial fork needs a fresh session"
    );
    fork.update(cx, |thread, cx| {
        report_control(
            thread,
            acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(vec![
                acp::AvailableCommand::new("help", "Show help"),
            ])),
            cx,
        );
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
    let id = cx.update(|cx| thread.read(cx).document().chat_id().to_owned());
    thread.update(cx, |thread, cx| {
        restore_entries(
            &f,
            thread,
            vec![nocterm_ai::thread::Entry::Agent("old answer".into())],
            cx,
        );
        restore_pending_history(&f, thread, cx);
        thread.send("interrupted question".into(), cx);
    });
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
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
    cx.update(|cx| assert!(restarted.read(cx).document().pending_history()));
    restarted.update(cx, |thread, cx| thread.send("try again".into(), cx));
    cx.run_until_parked();
    assert!(f.commands.prompts.lock().unwrap().last().unwrap().prompt.iter().any(
        |block| matches!(block, acp::ContentBlock::Text(text) if text.text.contains("Saved conversation") && text.text.contains("old answer"))
    ));
    complete_active(&f, cx);
    assert!(!disk_chat(&f, &id).pending_history);
}

fn adopt_json(
    f: &mut Fixture,
    chat: SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    cx.run_until_parked();
    f.repository.seed_chat(&chat).unwrap();
    let saved = f.repository.chats();
    assert_eq!(saved.len(), 1);
    reinstall_history(f, saved, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.adopt_saved_chats(window, cx);
            let thread = panel.chat_documents().next().unwrap().clone();
            panel.open_thread(thread.entity_id(), cx);
            thread
        })
    })
    .unwrap()
}

fn saved_chat(f: &Fixture, history: bool) -> SavedChat {
    let mut chat = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-4000-8000-000000000070".into(),
        1,
    );
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
    let mut f = fixture(cx);
    lazy_start(&f, cx);
    let saved = saved_chat(&f, false);
    let thread = adopt_json(&mut f, saved, cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    send_composer(&f, cx);
    assert!(
        f.commands.restores.lock().unwrap().is_empty(),
        "empty transcript tried to restore a provider rollout"
    );
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(thread.read(cx).session_generating());
        assert!(!thread.read(cx).has_status_error());
        assert_eq!(
            thread.read(cx).composer().queue()[0].saved.text,
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
    cx.update(|cx| assert!(thread.read(cx).composer().queue().is_empty()));
}

#[gpui_kit::test]
fn a_nonmissing_rpc_failure_does_not_replace_real_history_or_dispatch_its_queue(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    let saved = saved_chat(&f, true);
    let thread = adopt_json(&mut f, saved, cx);
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
        assert!(thread.read(cx).has_status_error());
        assert!(thread.read(cx).composer().paused());
        assert_eq!(thread.read(cx).composer().queue().len(), 2);
        assert_eq!(thread.read(cx).transcript().entries.len(), 1);
    });
}
