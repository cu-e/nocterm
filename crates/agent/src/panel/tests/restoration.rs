//! Empty provider sessions have no conversation to restore.
use super::*;
use nocterm_ai::model::commands::AvailableCommand;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::session::{PromptOutcome, SessionUpdate, StopReason};
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
                .is_some_and(|restore| restore.fork())
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
    assert!(
        prompts[0]
            .content
            .iter()
            .any(|block| matches!(block, Content::Text(text) if text.text == "waiting request"))
    );
    cx.update(|cx| assert!(!thread.read(cx).document().pending_history()));
}

#[gpui_kit::test]
fn missing_rollout_falls_back_once_and_preserves_history_and_queue(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let mut saved = empty_chat(&f);
    saved.entries = vec![
        Entry::User(vec![Content::text("remember this")]),
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
    assert!(prompts[0].content.iter().any(|block| matches!(block, Content::Text(text) if text.text.contains("User: remember this") && text.text.contains("Assistant: previous answer"))));
    assert!(
        prompts[0].content.iter().any(
            |block| matches!(block, Content::Text(text) if text.text == "continue conversation")
        )
    );
    drop(prompts);
    cx.update(|cx| assert_eq!(thread.read(cx).transcript().entries.len(), 3));
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| thread.send("next message".into(), cx));
    cx.run_until_parked();
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(!prompts[1].content.iter().any(|block| matches!(block, Content::Text(text) if text.text.contains("Saved conversation (context"))));
    assert_eq!(f.commands.restores.lock().unwrap().len(), 1);
}

pub(super) fn disk_chat(f: &Fixture, id: &str) -> nocterm_ai::history::SavedChat {
    f.repository
        .chats()
        .into_iter()
        .find(|chat| chat.header.id == id)
        .unwrap()
}

pub(super) fn restore_released(
    f: &Fixture,
    saved: nocterm_ai::history::SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    let retired = f.panel.update(cx, |panel, cx| {
        let threads = panel
            .chat_documents()
            .filter(|thread| thread.read(cx).document().chat_id() == saved.header.id)
            .cloned()
            .collect::<Vec<_>>();
        threads
            .into_iter()
            .map(|thread| {
                thread.update(cx, |thread, cx| thread.release_resources(cx));
                thread.entity_id()
            })
            .collect::<Vec<_>>()
    });
    cx.update(|cx| {
        for id in retired {
            f.installation
                .installed
                .runtime()
                .capture_and_detach_document(id, cx)
                .unwrap();
        }
    });
    cx.run_until_parked();
    cx.update(|cx| {
        f._directory
            .deps(
                f.workspace.downgrade(),
                f.installation.installed.clone(),
                cx,
            )
            .restore_chat(saved, cx)
            .unwrap()
    })
}
pub(super) fn reopen(
    f: &Fixture,
    saved: nocterm_ai::history::SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    let thread = restore_released(f, saved, cx);
    cx.update(|cx| activate_thread(&thread, cx));
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
            SessionUpdate::AvailableCommandsUpdate(vec![AvailableCommand::new(
                "help",
                "Show help",
            )]),
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
        .content
        .iter()
        .find_map(|block| match block {
            Content::Text(text) if text.text.contains("Saved conversation") => Some(&text.text),
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
        .send(PromptOutcome::new(StopReason::Cancelled))
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
    assert!(f.commands.prompts.lock().unwrap().last().unwrap().content.iter().any(
        |block| matches!(block, Content::Text(text) if text.text.contains("Saved conversation") && text.text.contains("old answer"))
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
        chat.entries = vec![nocterm_ai::thread::Entry::User(vec![Content::text(
            "prior real message",
        )])];
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
            thread.read(cx).composer().queue()[0].saved().text,
            "new submission"
        );
    });
    let text = |index: usize| {
        f.commands.prompts.lock().unwrap()[index]
            .content
            .iter()
            .filter_map(|block| match block {
                Content::Text(text) => Some(text.text.clone()),
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
            .push_back(AgentError::Rpc(nocterm_ai::model::session::RemoteError {
                code: -32603,
                message: "Service temporarily overloaded".into(),
            }));
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

fn catalog_row(
    f: &Fixture,
    chat: &SavedChat,
    cx: &mut TestAppContext,
) -> nocterm_agent_runtime::CatalogRow {
    f.repository.seed_chat(chat).unwrap();
    let runtime = f
        .installation
        .installed
        .runtime()
        .for_scope(chat.header.scope);
    cx.update(|cx| runtime.reload_history(cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let snapshot = runtime.catalog_snapshot(cx).unwrap();
        let index = snapshot
            .rows()
            .iter()
            .position(|row| row.id == chat.header.id)
            .unwrap();
        snapshot.row(index).unwrap()
    })
}

#[gpui_kit::test]
fn project_restore_refuses_before_entity_creation_and_preserves_the_general_chat(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap().clone());
    let before = cx.update(|cx| restored_projection(thread.read(cx), cx));
    let count = cx.update(|cx| f.panel.read(cx).chat_documents().count());
    let connects = f.connector.connects.load(Ordering::SeqCst);
    let mut project = before.clone();
    project.header.scope = serde_json::from_value(serde_json::json!({
        "kind": "project", "id": "01010101010101010101010101010101"
    }))
    .unwrap();
    project.draft = Some("foreign project draft".into());
    project.entries = vec![Entry::Agent("foreign project transcript".into())];
    let project_row = catalog_row(&f, &project, cx);
    cx.update(|cx| {
        let deps = f._directory.deps(
            f.workspace.downgrade(),
            f.installation.installed.clone(),
            cx,
        );
        assert!(matches!(
            deps.restore_chat(project.clone(), cx),
            Err(nocterm_agent_runtime::ChatOpenError::Storage(
                nocterm_ai::storage::AgentStorageError::ScopeMismatch { .. }
            ))
        ));
        assert!(matches!(
            deps.restore_archive(project_row, cx),
            Err(nocterm_agent_runtime::ChatOpenError::Storage(
                nocterm_ai::storage::AgentStorageError::ScopeMismatch { .. }
            ))
        ));
        let error = thread
            .update(cx, |thread, cx| thread.restore_content(project, cx))
            .unwrap_err();
        assert!(matches!(
            error,
            nocterm_ai::storage::AgentStorageError::ScopeMismatch { .. }
        ));
        let after = restored_projection(thread.read(cx), cx);
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        assert_eq!(f.panel.read(cx).chat_documents().count(), count);
    });
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), connects);
}

#[gpui_kit::test]
fn duplicate_restoration_keeps_the_original_document_and_reports_its_original_window(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let chat = empty_chat(&f);
    let (thread, deps) = cx.update(|cx| {
        let deps = f._directory.deps(
            f.workspace.downgrade(),
            f.installation.installed.clone(),
            cx,
        );
        let thread = deps.restore_chat(chat.clone(), cx).unwrap();
        (thread, deps)
    });
    let row = catalog_row(&f, &chat, cx);
    thread.update(cx, |thread, cx| {
        thread.set_draft("original document input".into(), cx)
    });
    cx.update(|cx| {
        assert!(matches!(deps.restore_chat(chat.clone(), cx), Err(nocterm_agent_runtime::ChatOpenError::AlreadyOpen { window, .. }) if window == f.workspace.entity_id()));
        assert!(matches!(deps.restore_archive(row, cx), Err(nocterm_agent_runtime::ChatOpenError::AlreadyOpen { window, .. }) if window == f.workspace.entity_id()));
        assert_eq!(thread.read(cx).snapshot(cx).unwrap().draft.as_deref(), Some("original document input"));
        assert!(deps.runtime().document_is_current(&chat.header.id, thread.entity_id(), cx));
    });
    cx.run_until_parked();
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 0);
}
#[gpui_kit::test]
fn restored_project_data_keeps_its_scope_after_edits_and_refuses_a_general_hydration(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let mut chat = empty_chat(&f);
    let scope = serde_json::from_value(
        serde_json::json!({"kind":"project", "id":"01010101010101010101010101010101"}),
    )
    .unwrap();
    chat.header.scope = scope;
    let original = chat.clone();
    let (thread, _surface, _outer) = cx
        .update_window(f.handle, |_, window, cx| {
            let general = f._directory.deps(
                f.workspace.downgrade(),
                f.installation.installed.clone(),
                cx,
            );
            let nocterm_ai::history::ContextId::Project(project) = scope else {
                panic!("project fixture");
            };
            let (seat, surface, outer) = super::seat_binding::project_seat(&f, project, window, cx);
            // This fixture exercises saved data without starting a local agent.
            let deps = crate::chat_deps::ChatDeps::new(
                general.runtime().for_scope(scope),
                general.agents().clone(),
                general.settings().clone(),
                f._directory.metadata.clone(),
                seat,
                None,
            );
            (deps.restore_chat(chat, cx).unwrap(), surface, outer)
        })
        .unwrap();
    thread.update(cx, |thread, cx| {
        thread.rename(Some("Scoped project title".into()), cx);
        thread.set_draft("project literal draft".into(), cx);
    });
    cx.update(|cx| {
        let before = thread.read(cx).snapshot(cx).unwrap();
        assert_eq!(before.header.scope, scope);
        assert_eq!(thread.read(cx).document().key().scope(), scope);
        let mut wrong = original;
        wrong.header.scope = nocterm_ai::history::ContextId::General;
        assert!(matches!(
            thread.update(cx, |thread, cx| thread.restore_content(wrong, cx)),
            Err(nocterm_ai::storage::AgentStorageError::ScopeMismatch { .. })
        ));
        let after = thread.read(cx).snapshot(cx).unwrap();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(before).unwrap()
        );
    });
    cx.run_until_parked();
    let id = cx.update(|cx| thread.read(cx).chat_id().to_owned());
    assert!(f.repository.encoded(&id).is_none());
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 0);
}
#[gpui_kit::test]
fn legacy_general_restore_edits_and_restart_keep_wire_version_while_forks_use_v2(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    let mut saved = empty_chat(&f);
    saved.version = 1;
    let original = adopt_saved(&mut f, saved, cx);
    original.update(cx, |thread, cx| {
        thread.rename(Some("Legacy title".into()), cx)
    });
    cx.update(|cx| assert_eq!(original.read(cx).snapshot(cx).unwrap().version, 1));
    let restarted = cx
        .update_window(f.handle, |_, window, cx| {
            f.panel.update(cx, |panel, cx| panel.restart(window, cx));
            f.panel.read(cx).current().unwrap()
        })
        .unwrap();
    cx.update(|cx| {
        assert_ne!(original.entity_id(), restarted.entity_id());
        assert_eq!(restarted.read(cx).snapshot(cx).unwrap().version, 1);
    });
    let fork = cx
        .update_window(f.handle, |_, window, cx| {
            f.panel.update(cx, |panel, cx| {
                panel.fork_thread(restarted.entity_id(), window, cx)
            });
            f.panel.read(cx).current().unwrap()
        })
        .unwrap();
    cx.update(|cx| assert_eq!(fork.read(cx).snapshot(cx).unwrap().version, 2));
}
