//! Lazy history, authority and metadata operations cross the runtime/UI boundary.
use super::*;
use nocterm_ai::{
    history::{SavedChat, SavedPrompt},
    thread::Entry,
};
fn archived(
    f: &Fixture,
    chat: &SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    f.repository.seed_chat(chat).unwrap();
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.reload_history(cx)));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        f.panel
            .read(cx)
            .chat_documents()
            .find(|thread| thread.read(cx).chat_id() == chat.id)
            .unwrap()
            .clone()
    })
}
fn enter_literal(f: &Fixture, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        let focus = f.panel.read(cx).composer_focus(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.into()));
        window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx);
    })
    .unwrap();
    cx.run_until_parked();
}
fn saved() -> SavedChat {
    let mut chat = SavedChat::new("codex".into());
    chat.entries.push(Entry::Agent("historic answer".into()));
    chat
}
#[gpui_kit::test]
fn catalog_rows_are_idle_and_opening_hydrates_and_connects_with_the_queue_paused(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chat = saved();
    chat.draft = Some("literal draft".into());
    chat.queue.push(SavedPrompt {
        id: 8,
        text: "pending".into(),
        images: vec![],
        attachments: vec![],
    });
    let thread = archived(&f, &chat, cx);
    let updates = Rc::new(std::cell::Cell::new(0));
    let seen = updates.clone();
    let _observe = cx.update(|cx| cx.observe(&thread, move |_, _| seen.set(seen.get() + 1)));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(updates.get(), 0);
    cx.update(|cx| {
        assert!(thread.read(cx).is_archived());
        assert!(thread.read(cx).history_entries().is_empty());
        assert!(thread.read(cx).flush_snapshot(cx).is_none());
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert!(!thread.is_archived());
        assert_eq!(thread.history_entries().len(), 1);
        assert_eq!(thread.draft_text(), Some("literal draft"));
        assert_eq!(thread.queued_prompt_ids().next().unwrap(), 8);
        // Opened, the chat connects so its pickers work, but the queue it
        // was saved with waits for the user.
        assert_eq!(
            thread.session_phase(),
            nocterm_ai::session::SessionPhase::Ready
        );
        assert!(!thread.queue_dispatchable());
    });
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}
#[gpui_kit::test]
fn rename_and_pin_hydrate_before_writing_the_archive(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let chat = saved();
    let thread = archived(&f, &chat, cx);
    thread.update(cx, |thread, cx| {
        thread.rename(Some("New name".into()), cx);
        thread.set_pinned(true, cx);
    });
    cx.run_until_parked();
    cx.update(|_| {
        let loaded = f.repository.transcript(&chat.id).unwrap();
        assert_eq!(loaded.name.as_deref(), Some("New name"));
        assert!(loaded.pinned);
        assert_eq!(loaded.entries.len(), 1);
    });
}
#[gpui_kit::test]
fn delete_while_loading_does_not_resurrect_the_document(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let chat = saved();
    let thread = archived(&f, &chat, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.open_thread(thread.entity_id(), cx);
            panel.delete_thread(thread.entity_id(), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(f.repository.transcript(&chat.id).is_err());
        assert!(
            f.panel
                .read(cx)
                .chat_documents()
                .all(|thread| thread.read(cx).chat_id() != chat.id)
        );
    });
}
#[gpui_kit::test]
fn restored_root_controls_process_sandbox_restore_and_fresh_fallback(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let a = f._directory.path().join("A");
    let b = f._directory.path().join("B");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let global = b.to_string_lossy().into_owned();
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(move |settings| {
            settings.working_directory = Some(global);
            settings.sandbox = nocterm_ai::SandboxMode::Workspace;
        })
        .detach();
    });
    cx.run_until_parked();
    let mut chat = saved();
    chat.workdir = Some(a.clone());
    chat.session_id = Some("previous-session".into());
    let thread = archived(&f, &chat, cx);
    // Opening warms the chat, which restores it right away.
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::RestoreUnavailable("no rollout".into()));
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| thread.send("continue".into(), cx));
    cx.run_until_parked();
    assert_eq!(
        f.commands.restore_dirs.lock().unwrap().as_slice(),
        std::slice::from_ref(&a)
    );
    assert_eq!(
        f.commands.new_dirs.lock().unwrap().as_slice(),
        std::slice::from_ref(&a)
    );
    let roots = f.connector.roots.lock().unwrap();
    assert_eq!(roots[0].0, a);
    let policy = roots[0].1.as_ref().expect("workspace sandbox");
    assert!(policy.writable.contains(&a));
    assert!(!policy.writable.contains(&b));
}
#[gpui_kit::test]
fn hydration_waits_for_shared_permit_and_clean_archives_are_evicted(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chats = Vec::new();
    for index in 0..15 {
        let mut chat = saved();
        chat.updated = index;
        chat.entries = (0..2000).map(|_| Entry::Agent("row".into())).collect();
        chats.push(chat);
    }
    for chat in &chats {
        f.repository.seed_chat(chat).unwrap();
    }
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(
            chats.first().unwrap().id.clone(),
        ));
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.reload_history(cx)));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let threads = cx.update(|cx| {
        f.panel
            .read(cx)
            .chat_documents()
            .cloned()
            .collect::<Vec<_>>()
    });
    f.panel.update(cx, |panel, cx| {
        panel.open_thread(threads[0].entity_id(), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(threads[0].read(cx).is_archived()));
    f.panel.update(cx, |panel, cx| {
        panel.open_thread(threads[1].entity_id(), cx)
    });
    cx.run_until_parked();
    let loads = f
        .repository
        .operations()
        .into_iter()
        .filter(|operation| matches!(operation, nocterm_ai::testing::StorageOperation::Load(_)))
        .count();
    assert_eq!(
        loads, 1,
        "a second payload crossed the shared hydration gate"
    );
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
    cx.update(|cx| assert!(!threads[0].read(cx).is_archived()));
    for thread in &threads[1..] {
        f.panel
            .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
        cx.run_until_parked();
    }
    cx.update(|cx| {
        let resident: usize = threads
            .iter()
            .map(|thread| thread.read(cx).resident_history_bytes())
            .sum();
        assert!(resident <= 64 * 1024 * 1024, "resident {resident}");
        assert!(threads.iter().any(|thread| thread.read(cx).is_archived()));
        assert!(!threads.last().unwrap().read(cx).is_archived());
    });
}
#[gpui_kit::test]
fn typing_while_archive_loading_preserves_latest_literal_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chat = saved();
    chat.updated = 1;
    chat.draft = Some("old saved draft".into());
    let thread = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    enter_literal(&f, "new literal\ndraft", cx);
    cx.run_until_parked();
    let edited_at = cx.update(|cx| {
        let thread = thread.read(cx);
        assert!(thread.is_archived());
        assert!(thread.updated_at() > chat.updated);
        thread.updated_at()
    });
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "new literal\ndraft"
        );
        assert_eq!(thread.read(cx).draft_text(), Some("new literal\ndraft"));
        assert_eq!(thread.read(cx).updated_at(), edited_at);
        let loaded = f.repository.transcript(&chat.id).unwrap();
        assert_eq!(loaded.draft.as_deref(), Some("new literal\ndraft"));
        assert_eq!(loaded.updated, edited_at);
    });
}
#[gpui_kit::test]
fn disable_while_loading_saves_draft_patch_without_truncating_archive(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chat = saved();
    chat.queue.push(SavedPrompt {
        id: 3,
        text: "queued request".into(),
        images: vec![],
        attachments: vec![],
    });
    let thread = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    enter_literal(&f, "draft before disable", cx);
    cx.run_until_parked();
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
            .detach()
    });
    cx.run_until_parked();
    cx.update(|_| {
        let saved = f.repository.transcript(&chat.id).unwrap();
        assert_eq!(saved.draft.as_deref(), Some("draft before disable"));
        assert_eq!(saved.entries.len(), 1);
        assert_eq!(saved.queue[0].id, 3);
    });
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
}
#[gpui_kit::test]
fn native_quit_flushes_pending_archived_draft_with_history_gate_held(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chat = saved();
    chat.name = Some("original name".into());
    chat.queue.push(SavedPrompt {
        id: 4,
        text: "pending queue".into(),
        images: vec![],
        attachments: vec![],
    });
    let thread = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    enter_literal(&f, "draft at quit", cx);
    cx.run_until_parked();
    cx.update(App::shutdown);
    let saved = f.repository.transcript(&chat.id).unwrap();
    assert_eq!(saved.draft.as_deref(), Some("draft at quit"));
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.queue[0].id, 4);
    assert_eq!(saved.name.as_deref(), Some("original name"));
    let _ = blocked.send(Ok(()));
}
#[gpui_kit::test]
fn invalid_saved_root_fails_safely_and_legacy_missing_root_starts_fresh(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut invalid = saved();
    invalid.session_id = Some("old-session".into());
    invalid.workdir = Some(f._directory.path().join("removed-project"));
    let thread = archived(&f, &invalid, cx);
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| thread.send("continue".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    cx.update(|cx| assert!(thread.read(cx).has_status_error()));
    let mut legacy = saved();
    legacy.session_id = Some("old-without-directory".into());
    let thread = archived(&f, &legacy, cx);
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| thread.send("continue legacy".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    assert!(f.commands.restores.lock().unwrap().is_empty());
    let roots = f.connector.roots.lock().unwrap();
    assert_eq!(f.commands.new_dirs.lock().unwrap()[0], roots[0].0);
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).session_workdir().as_ref().unwrap(),
            &roots[0].0
        )
    });
}

#[gpui_kit::test]
fn failed_hydration_retries_with_only_new_metadata_intentions(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let chat = saved();
    let thread = archived(&f, &chat, cx);
    f.repository.fail_next(
        nocterm_ai::testing::StorageOperation::Load(chat.id.clone()),
        nocterm_ai::storage::AgentStorageError::Interrupted,
    );
    thread.update(cx, |thread, cx| {
        thread.rename(Some("failed name".into()), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(thread.read(cx).is_archived()));
    thread.update(cx, |thread, cx| thread.set_pinned(true, cx));
    cx.run_until_parked();
    let durable = f.repository.transcript(&chat.id).unwrap();
    assert_eq!(durable.name, chat.name);
    assert!(durable.pinned);
    assert_eq!(durable.entries.len(), 1);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn deferred_fork_uses_original_chat_and_row_after_active_chat_changes(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chat = saved();
    chat.name = Some("original source".into());
    chat.entries.push(Entry::Agent("later answer".into()));
    let source = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread_at(source.entity_id(), Some(0), window, cx);
            panel.new_thread("codex".into(), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
    let fork = f
        .repository
        .chats()
        .into_iter()
        .find(|saved| saved.name.as_deref() == Some("original source (fork)"))
        .expect("original source was forked");
    assert_ne!(fork.id, chat.id);
    assert_eq!(fork.entries.len(), 1);
    assert!(matches!(&fork.entries[0], Entry::Agent(text) if text == "historic answer"));
    assert_eq!(f.repository.transcript(&chat.id).unwrap().entries.len(), 2);
}

#[gpui_kit::test]
fn deleted_source_does_not_fork_after_deferred_hydration(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let chat = saved();
    let source = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread(source.entity_id(), window, cx);
            panel.delete_thread(source.entity_id(), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
    assert!(f.repository.chats().is_empty());
    cx.update(|cx| assert_eq!(f.panel.read(cx).chat_documents().len(), 0));
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn clearing_native_input_while_hydration_waits_wins_over_saved_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chat = saved();
    chat.draft = Some("old persisted draft".into());
    let source = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    f.panel
        .update(cx, |panel, cx| panel.open_thread(source.entity_id(), cx));
    cx.run_until_parked();
    enter_literal(&f, "temporary text", cx);
    cx.simulate_keystrokes(f.handle, "ctrl-a backspace");
    cx.run_until_parked();
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
    let loaded = f.repository.transcript(&chat.id).unwrap();
    assert!(loaded.draft.is_none());
    assert_eq!(loaded.entries.len(), 1);
    cx.update(|cx| {
        assert!(source.read(cx).draft_text().is_none());
        assert!(f.panel.read(cx).composer_text(cx).is_empty());
    });
}

#[gpui_kit::test]
fn hydration_observer_can_rename_then_delete_without_reentering_owner_borrow(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.run_until_parked();
    let chat = saved();
    let source = archived(&f, &chat, cx);
    let panel = f.panel.clone();
    let window = f.handle;
    let once = Rc::new(std::cell::Cell::new(false));
    let observed = once.clone();
    let _subscription = cx.update(|cx| {
        cx.observe(&source, move |thread, cx| {
            if thread.read(cx).is_archived() || observed.replace(true) {
                return;
            }
            let id = thread.entity_id();
            thread.update(cx, |thread, cx| {
                thread.rename(Some("observer name".into()), cx)
            });
            let _ = window.update(cx, |_, window, cx| {
                panel.update(cx, |panel, cx| panel.delete_thread(id, window, cx));
            });
        })
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(source.entity_id(), cx));
    cx.run_until_parked();
    assert!(once.get());
    assert!(f.repository.transcript(&chat.id).is_err());
    cx.update(|cx| assert_eq!(f.panel.read(cx).chat_documents().len(), 0));
}

#[gpui_kit::test]
fn closed_original_window_does_not_fork_after_hydration(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let chat = saved();
    let source = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread(source.entity_id(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
    let saved = f.repository.chats();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, chat.id);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn blocked_open_applies_metadata_intentions_once_before_warm_connection(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let mut chat = saved();
    chat.queue.push(SavedPrompt {
        id: 9,
        text: "held queue".into(),
        images: vec![],
        attachments: vec![],
    });
    let source = archived(&f, &chat, cx);
    let blocked = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(chat.id.clone()));
    f.panel
        .update(cx, |panel, cx| panel.open_thread(source.entity_id(), cx));
    cx.run_until_parked();
    source.update(cx, |thread, cx| {
        thread.rename(Some("first accepted name".into()), cx);
        thread.set_pinned(true, cx);
        thread.rename(Some("latest accepted name".into()), cx);
    });
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    let _ = blocked.send(Ok(()));
    cx.run_until_parked();
    let loaded = f.repository.transcript(&chat.id).unwrap();
    assert_eq!(loaded.name.as_deref(), Some("latest accepted name"));
    assert!(loaded.pinned);
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.queue[0].id, 9);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    assert!(f.commands.prompts.lock().unwrap().is_empty());
    let loads = f.repository.operations().iter().filter(|operation|
        matches!(operation, nocterm_ai::testing::StorageOperation::Load(id) if id == &chat.id)).count();
    assert_eq!(loads, 1);
}
