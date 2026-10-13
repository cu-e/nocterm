use super::*;

#[gpui_kit::test]
fn native_quit_preserves_the_latest_document_after_destroying_its_window(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.set_draft("latest dormant document".into(), cx);
    });
    let weak = thread.downgrade();
    drop(thread);
    let Fixture {
        panel,
        workspace,
        repository,
        ..
    } = f;
    drop(panel);
    drop(workspace);
    // Exercise GPUI's native shutdown: quit callbacks, window destruction, then bounded task drain.
    cx.update(App::shutdown);
    assert!(
        weak.upgrade().is_none(),
        "test accidentally retained the document past window destruction"
    );
    let chats = repository.chats();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].draft.as_deref(), Some("latest dormant document"));
}

#[gpui_kit::test]
fn shutdown_is_durable_even_if_its_task_is_dropped_and_old_writes_are_pending(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.set_draft("original content".into(), cx);
        thread.rename(Some("old snapshot".into()), cx);
        thread.save(cx);
    });
    thread.update(cx, |thread, cx| {
        thread.rename(Some("latest unsaved document".into()), cx)
    });
    // Native GPUI may drop this cleanup task after its 200ms grace period.
    // Durability must already hold when the synchronous quit callback returns.
    shutdown_runtime(cx);
    let read = || f.repository.chats();
    assert_eq!(read().len(), 1);
    assert_eq!(read()[0].name.as_deref(), Some("latest unsaved document"));
    cx.run_until_parked();
    assert_eq!(
        read()[0].name.as_deref(),
        Some("latest unsaved document"),
        "a stale background snapshot overwrote synchronous quit persistence"
    );
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}
