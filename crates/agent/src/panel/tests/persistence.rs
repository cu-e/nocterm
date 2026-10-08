//! Durability at the boundary between document lifetime and asynchronous disk writes.
use super::*;

fn document(f: &Fixture, cx: &mut TestAppContext) -> Entity<crate::thread::AgentThread> {
    cx.run_until_parked();
    let thread = cx
        .update_window(f.handle, |_, window, cx| {
            f.panel
                .update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx));
            f.panel.read(cx).current().unwrap()
        })
        .unwrap();
    thread.update(cx, |thread, _| {
        thread
            .state
            .push_user(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "persist this document",
            ))]);
    });
    cx.run_until_parked();
    thread
}

fn remove_document(
    f: &Fixture,
    thread: Entity<crate::thread::AgentThread>,
    cx: &mut TestAppContext,
) {
    let weak = thread.downgrade();
    let id = thread.entity_id();
    f.panel.update(cx, |panel, cx| {
        panel.retain_threads(|candidate, _| candidate.entity_id() != id, cx)
    });
    drop(thread);
    cx.update(|_| {});
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn shutdown_preserves_a_snapshot_extracted_by_the_writer_after_document_release(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = document(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.name = Some("pending extracted snapshot".into());
        thread.save(cx);
    });
    assert!(cx.background_executor.tick());
    let dir = f._directory.path().join("chats");
    assert!(
        nocterm_ai::history::load_all(&dir).is_empty(),
        "background IO ran before the shutdown interleaving"
    );
    remove_document(&f, thread, cx);
    cx.update(|cx| drop(Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))));
    let chats = nocterm_ai::history::load_all(&dir);
    assert_eq!(
        chats.len(),
        1,
        "quit lost the writer-owned snapshot after its document was released"
    );
    assert_eq!(chats[0].name.as_deref(), Some("pending extracted snapshot"));
    cx.run_until_parked();
    assert_eq!(
        nocterm_ai::history::load_all(&dir)[0].name.as_deref(),
        Some("pending extracted snapshot")
    );
}

#[gpui_kit::test]
fn shutdown_preserves_a_delete_extracted_by_the_writer_after_document_release(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = document(&f, cx);
    thread.update(cx, |thread, cx| thread.save(cx));
    cx.run_until_parked();
    let dir = f._directory.path().join("chats");
    assert_eq!(nocterm_ai::history::load_all(&dir).len(), 1);
    let id = cx.update(|cx| thread.read(cx).chat_id.clone());
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.delete_chat(id, cx)));
    assert!(cx.background_executor.tick());
    assert_eq!(
        nocterm_ai::history::load_all(&dir).len(),
        1,
        "delete IO ran before the shutdown interleaving"
    );
    remove_document(&f, thread, cx);
    cx.update(|cx| drop(Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))));
    assert!(
        nocterm_ai::history::load_all(&dir).is_empty(),
        "quit lost the writer-owned deletion"
    );
    cx.run_until_parked();
    assert!(nocterm_ai::history::load_all(&dir).is_empty());
}

#[gpui_kit::test]
fn shutdown_saves_the_replacement_when_the_old_document_is_retained_elsewhere(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let old = document(&f, cx);
    old.update(cx, |thread, cx| {
        thread.name = Some("old document".into());
        thread.save(cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_ne!(old.entity_id(), replacement.entity_id());
    cx.update(|cx| assert_eq!(old.read(cx).chat_id, replacement.read(cx).chat_id));
    replacement.update(cx, |thread, _| {
        thread.name = Some("replacement wins".into())
    });
    cx.update(|cx| drop(Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))));
    let chats = nocterm_ai::history::load_all(&f._directory.path().join("chats"));
    assert_eq!(chats.len(), 1);
    assert_eq!(
        chats[0].name.as_deref(),
        Some("replacement wins"),
        "an externally retained old document overwrote its replacement"
    );
    // Keep the stale entity alive through the assertion, as an outstanding callback can do.
    cx.update(|cx| assert_eq!(old.read(cx).name.as_deref(), Some("old document")));
}

#[gpui_kit::test]
fn a_late_save_from_a_retained_old_document_cannot_overwrite_the_replacement(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let old = document(&f, cx);
    old.update(cx, |thread, cx| {
        thread.name = Some("original owner".into());
        thread.save(cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    replacement.update(cx, |thread, cx| {
        thread.name = Some("replacement owner".into());
        thread.save(cx);
    });
    cx.run_until_parked();
    let dir = f._directory.path().join("chats");
    assert_eq!(
        nocterm_ai::history::load_all(&dir)[0].name.as_deref(),
        Some("replacement owner")
    );
    // An outstanding callback can retain the old Entity and eventually attempt to save it.
    old.update(cx, |thread, cx| {
        thread.name = Some("late obsolete callback".into());
        thread.save(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        nocterm_ai::history::load_all(&dir)[0].name.as_deref(),
        Some("replacement owner"),
        "a stale owner saved over the replacement outside shutdown"
    );
    cx.update(|cx| drop(Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))));
    assert_eq!(
        nocterm_ai::history::load_all(&dir)[0].name.as_deref(),
        Some("replacement owner")
    );
}

#[gpui_kit::test]
fn shutdown_prefers_the_latest_queued_snapshot_over_the_extracted_older_snapshot(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = document(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.name = Some("extracted old snapshot".into());
        thread.save(cx);
    });
    assert!(cx.background_executor.tick());
    let dir = f._directory.path().join("chats");
    assert!(nocterm_ai::history::load_all(&dir).is_empty());
    thread.update(cx, |thread, cx| {
        thread.name = Some("latest queued snapshot".into());
        thread.save(cx);
    });
    remove_document(&f, thread, cx);
    cx.update(|cx| drop(Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))));
    let chats = nocterm_ai::history::load_all(&dir);
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].name.as_deref(), Some("latest queued snapshot"));
    cx.run_until_parked();
    assert_eq!(
        nocterm_ai::history::load_all(&dir)[0].name.as_deref(),
        Some("latest queued snapshot")
    );
}
