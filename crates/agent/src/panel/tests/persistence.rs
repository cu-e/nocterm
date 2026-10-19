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
    thread.update(cx, |thread, cx| {
        thread.set_draft("persist this document".into(), cx);
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
    lazy_start(&f, cx);
    let thread = document(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.rename(Some("pending extracted snapshot".into()), cx);
        thread.save(cx);
    });
    assert!(cx.background_executor.tick());
    assert!(
        f.repository.chats().is_empty(),
        "background IO ran before the shutdown interleaving"
    );
    remove_document(&f, thread, cx);
    shutdown_runtime(&f.installation.installed, cx);
    let chats = f.repository.chats();
    assert_eq!(
        chats.len(),
        1,
        "quit lost the writer-owned snapshot after its document was released"
    );
    assert_eq!(
        chats[0].metadata().name.as_deref(),
        Some("pending extracted snapshot")
    );
    cx.run_until_parked();
    assert_eq!(
        f.repository.chats()[0].metadata().name.as_deref(),
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
    assert_eq!(f.repository.chats().len(), 1);
    let id = cx.update(|cx| thread.read(cx).chat_id_string());
    cx.update(|cx| f.installation.installed.runtime().delete_chat(id, cx));
    assert!(cx.background_executor.tick());
    assert_eq!(
        f.repository.chats().len(),
        1,
        "delete IO ran before the shutdown interleaving"
    );
    remove_document(&f, thread, cx);
    shutdown_runtime(&f.installation.installed, cx);
    assert!(
        f.repository.chats().is_empty(),
        "quit lost the writer-owned deletion"
    );
    cx.run_until_parked();
    assert!(f.repository.chats().is_empty());
}

#[gpui_kit::test]
fn shutdown_saves_the_replacement_when_the_old_document_is_retained_elsewhere(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let old = document(&f, cx);
    old.update(cx, |thread, cx| {
        thread.rename(Some("old document".into()), cx);
        thread.save(cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_ne!(old.entity_id(), replacement.entity_id());
    replacement.update(cx, |thread, cx| {
        thread.rename(Some("replacement wins".into()), cx)
    });
    shutdown_runtime(&f.installation.installed, cx);
    let chats = f.repository.chats();
    assert_eq!(chats.len(), 1);
    cx.update(|cx| assert_eq!(chats[0].id(), replacement.read(cx).chat_id_string()));
    assert_eq!(
        chats[0].metadata().name.as_deref(),
        Some("replacement wins"),
        "an externally retained old document overwrote its replacement"
    );
    // Keep the stale entity alive through the assertion, as an outstanding callback can do.
    cx.update(|cx| assert!(old.read(cx).document().name().is_none()));
}

#[gpui_kit::test]
fn a_late_save_from_a_retained_old_document_cannot_overwrite_the_replacement(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let old = document(&f, cx);
    old.update(cx, |thread, cx| {
        thread.rename(Some("original owner".into()), cx);
        thread.save(cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    replacement.update(cx, |thread, cx| {
        thread.rename(Some("replacement owner".into()), cx);
        thread.save(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        f.repository.chats()[0].metadata().name.as_deref(),
        Some("replacement owner")
    );
    // An outstanding callback can retain the old Entity and eventually attempt to save it.
    old.update(cx, |thread, cx| {
        thread.rename(Some("late obsolete callback".into()), cx);
        thread.save(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        f.repository.chats()[0].metadata().name.as_deref(),
        Some("replacement owner"),
        "a stale owner saved over the replacement outside shutdown"
    );
    shutdown_runtime(&f.installation.installed, cx);
    assert_eq!(
        f.repository.chats()[0].metadata().name.as_deref(),
        Some("replacement owner")
    );
}

#[gpui_kit::test]
fn shutdown_prefers_the_latest_queued_snapshot_over_the_extracted_older_snapshot(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let thread = document(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.rename(Some("extracted old snapshot".into()), cx);
        thread.save(cx);
    });
    assert!(cx.background_executor.tick());
    assert!(f.repository.chats().is_empty());
    thread.update(cx, |thread, cx| {
        thread.rename(Some("latest queued snapshot".into()), cx);
        thread.save(cx);
    });
    remove_document(&f, thread, cx);
    shutdown_runtime(&f.installation.installed, cx);
    let chats = f.repository.chats();
    assert_eq!(chats.len(), 1);
    assert_eq!(
        chats[0].metadata().name.as_deref(),
        Some("latest queued snapshot")
    );
    cx.run_until_parked();
    assert_eq!(
        f.repository.chats()[0].metadata().name.as_deref(),
        Some("latest queued snapshot")
    );
}

fn save_count(f: &Fixture, id: &str) -> usize {
    f.repository.operations().iter().filter(|operation|matches!(operation,nocterm_ai::testing::StorageOperation::Save(saved) if saved==id)).count()
}
#[gpui_kit::test]
fn loaded_browse_noop_actions_detach_and_quit_preserve_the_original_v1_bytes(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let saved = nocterm_ai::history::SavedChat::for_identity(
        &f.installation.retention,
        "codex".into(),
        "00000000-0000-4000-8000-000000000002".into(),
        40,
    );
    let (id, metadata, mut resume, mut transcript, composer, _) = saved.into_parts();
    transcript.entries = vec![nocterm_ai::thread::Entry::Agent("old answer".into())];
    transcript.times = vec![39];
    resume.pending_history = true;
    let saved = nocterm_ai::history::SavedChat::from_parts(
        &f.installation.retention,
        id,
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    let thread = super::archive::archived(&f, &saved, cx);
    let bytes = f.repository.encoded(saved.id()).unwrap();
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.rename(None, cx);
        thread.set_pinned(false, cx);
        thread.set_draft(String::new(), cx);
        assert!(thread.shared_snapshot(cx).is_some());
        assert!(thread.flush_snapshot(cx).is_none());
        thread.release_resources(cx);
    });
    cx.run_until_parked();
    shutdown_runtime(&f.installation.installed, cx);
    assert_eq!(save_count(&f, saved.id()), 0);
    assert_eq!(f.repository.encoded(saved.id()).unwrap(), bytes);
    cx.update(|cx| assert_eq!(thread.read(cx).updated_at(), 40));
}
#[gpui_kit::test]
fn equal_warm_resume_does_not_rewrite_imported_session_metadata(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let saved = nocterm_ai::history::SavedChat::for_identity(
        &f.installation.retention,
        "codex".into(),
        "00000000-0000-4000-8000-000000000003".into(),
        40,
    );
    let (id, metadata, mut resume, mut transcript, composer, _) = saved.into_parts();
    transcript.entries = vec![nocterm_ai::thread::Entry::Agent("old answer".into())];
    resume.session_id = Some("resumed".into());
    resume.workdir = Some("/tmp".into());
    let saved = nocterm_ai::history::SavedChat::from_parts(
        &f.installation.retention,
        id,
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    let thread = super::archive::archived(&f, &saved, cx);
    let bytes = f.repository.encoded(saved.id()).unwrap();
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).session().as_ref().map(ToString::to_string),
            Some("resumed".into())
        )
    });
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    shutdown_runtime(&f.installation.installed, cx);
    assert_eq!(save_count(&f, saved.id()), 0);
    assert_eq!(f.repository.encoded(saved.id()).unwrap(), bytes);
}
#[gpui_kit::test]
fn an_old_disk_failure_before_new_draft_admission_does_not_pause_the_new_desire(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let thread = super::resources::draft(&f, cx);
    let id = cx.update(|cx| thread.read(cx).chat_id_string());
    let release = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Save(id.clone()));
    thread.update(cx, |thread, cx| {
        thread.rename(Some("first snapshot".into()), cx)
    });
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        thread.set_draft("latest literal draft".into(), cx)
    });
    release
        .send(Err(nocterm_ai::storage::AgentStorageError::Interrupted))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(thread.read(cx).document().persistence_error().is_none());
        assert!(!thread.read(cx).composer().paused());
        assert_eq!(
            thread.read(cx).draft_text().as_deref(),
            Some("latest literal draft")
        );
    });
    shutdown_runtime(&f.installation.installed, cx);
    assert_eq!(
        f.repository
            .transcript(&id)
            .unwrap()
            .composer()
            .draft
            .as_deref(),
        Some("latest literal draft")
    );
}
#[gpui_kit::test]
fn shutdown_before_the_draft_timer_saves_the_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let thread = super::resources::draft(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.set_draft("before 300 ms".into(), cx)
    });
    let id = cx.update(|cx| thread.read(cx).chat_id_string());
    cx.update(|cx| assert!(thread.read(cx).shared_snapshot(cx).is_some()));
    // Repeated captures before the timer share one revision.
    let first = thread.update(cx, |thread, cx| thread.capture_pending(cx).unwrap());
    let second = thread.update(cx, |thread, cx| thread.capture_pending(cx).unwrap());
    assert!(first.is_some());
    assert_eq!(first, second);
    shutdown_runtime(&f.installation.installed, cx);
    assert_eq!(
        f.repository
            .transcript(&id)
            .unwrap()
            .composer()
            .draft
            .as_deref(),
        Some("before 300 ms")
    );
    cx.update(|cx| assert_eq!(Some(thread.read(cx).document().desired_revision()), first));
}
#[gpui_kit::test]
fn shared_snapshots_reuse_queued_payload_and_cache_exact_size(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("active".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        attach_image(thread, image_bytes(), image_codec(), cx);
        thread.send("queued\n\"\\界".into(), cx);
        let saved = thread.snapshot(cx).unwrap();
        let queued = &saved.composer().queue[0];
        assert_eq!(queued.text, "queued\n\"\\界");
        assert_eq!(queued.images.len(), 1);
        assert_eq!(thread.composer().queue()[0].images().len(), 1);
        assert_eq!(thread.composer().queue()[0].text().to_string(), queued.text);
    });
}
