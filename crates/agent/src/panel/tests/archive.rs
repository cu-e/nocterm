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
    let rows = cx.update(|cx| {
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        nocterm_ai::history::save(&dir, chat).unwrap();
        nocterm_ai::history::load_catalog(&dir)
    });
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, _| runtime.saved_catalog = Some(rows)));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        f.panel
            .read(cx)
            .threads
            .iter()
            .find(|thread| thread.read(cx).chat_id == chat.id)
            .unwrap()
            .clone()
    })
}
fn saved() -> SavedChat {
    let mut chat = SavedChat::new("codex".into());
    chat.entries.push(Entry::Agent("historic answer".into()));
    chat
}
#[gpui_kit::test]
fn catalog_rows_are_idle_and_opening_hydrates_draft_and_queue(cx: &mut TestAppContext) {
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
        assert!(thread.read(cx).archive.is_some());
        assert!(thread.read(cx).state.entries.is_empty());
        assert!(thread.read(cx).flush_snapshot(cx).is_none());
    });
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert!(thread.archive.is_none());
        assert_eq!(thread.state.entries.len(), 1);
        assert_eq!(thread.composer.draft.as_deref(), Some("literal draft"));
        assert_eq!(thread.composer.queue[0].saved.id, 8);
    });
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
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
    cx.update(|cx| {
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        let loaded = nocterm_ai::history::load(&dir, &chat.id).unwrap();
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
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        assert!(nocterm_ai::history::load(&dir, &chat.id).is_err());
        assert!(
            f.panel
                .read(cx)
                .threads
                .iter()
                .all(|thread| thread.read(cx).chat_id != chat.id)
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
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::RestoreUnavailable("no rollout".into()));
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
    let gate = cx.update(|cx| Runtime::global(cx).read(cx).history_gate.clone());
    let permit = futures::executor::block_on(gate.acquire());
    let mut chats = Vec::new();
    for index in 0..15 {
        let mut chat = saved();
        chat.updated = index;
        chat.entries = (0..2000).map(|_| Entry::Agent("row".into())).collect();
        chats.push(chat);
    }
    let rows = cx.update(|cx| {
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        for chat in &chats {
            nocterm_ai::history::save(&dir, chat).unwrap();
        }
        nocterm_ai::history::load_catalog(&dir)
    });
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, _| runtime.saved_catalog = Some(rows)));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let threads = cx.update(|cx| f.panel.read(cx).threads.clone());
    f.panel.update(cx, |panel, cx| {
        panel.open_thread(threads[0].entity_id(), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(threads[0].read(cx).archive.is_some()));
    drop(permit);
    cx.run_until_parked();
    cx.update(|cx| assert!(threads[0].read(cx).archive.is_none()));
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
        assert!(
            threads
                .iter()
                .any(|thread| thread.read(cx).archive.is_some())
        );
        assert!(threads.last().unwrap().read(cx).archive.is_none());
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
    let gate = cx.update(|cx| Runtime::global(cx).read(cx).history_gate.clone());
    let permit = futures::executor::block_on(gate.acquire());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.input.update(cx, |input, cx| {
                input.set_value("new literal\ndraft", window, cx)
            })
        });
    })
    .unwrap();
    cx.run_until_parked();
    let edited_at = cx.update(|cx| {
        let thread = thread.read(cx);
        assert!(thread.archive.is_some());
        assert!(thread.updated > chat.updated);
        thread.updated
    });
    drop(permit);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "new literal\ndraft"
        );
        assert_eq!(
            thread.read(cx).composer.draft.as_deref(),
            Some("new literal\ndraft")
        );
        assert_eq!(thread.read(cx).updated, edited_at);
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        let loaded = nocterm_ai::history::load(&dir, &chat.id).unwrap();
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
    let gate = cx.update(|cx| Runtime::global(cx).read(cx).history_gate.clone());
    let permit = futures::executor::block_on(gate.acquire());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.input.update(cx, |input, cx| {
                input.set_value("draft before disable", window, cx)
            })
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
            .detach()
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        let saved = nocterm_ai::history::load(&dir, &chat.id).unwrap();
        assert_eq!(saved.draft.as_deref(), Some("draft before disable"));
        assert_eq!(saved.entries.len(), 1);
        assert_eq!(saved.queue[0].id, 3);
    });
    drop(permit);
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
    let (gate, dir) = cx.update(|cx| {
        let runtime = Runtime::global(cx);
        (
            runtime.read(cx).history_gate.clone(),
            runtime.read(cx).services.chats_dir.clone(),
        )
    });
    let permit = futures::executor::block_on(gate.acquire());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel
                .input
                .update(cx, |input, cx| input.set_value("draft at quit", window, cx))
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(App::shutdown);
    let saved = nocterm_ai::history::load(&dir, &chat.id).unwrap();
    assert_eq!(saved.draft.as_deref(), Some("draft at quit"));
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.queue[0].id, 4);
    assert_eq!(saved.name.as_deref(), Some("original name"));
    drop(permit);
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
    cx.update(|cx| assert!(thread.read(cx).status_error));
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
