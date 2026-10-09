use super::*;

#[gpui_kit::test]
fn native_quit_preserves_the_latest_document_after_destroying_its_window(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, _| {
        thread
            .state
            .push_user(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "latest dormant document",
            ))]);
    });
    let weak = thread.downgrade();
    drop(thread);
    let Fixture {
        panel,
        workspace,
        _directory: directory,
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
    let chats = nocterm_ai::history::load_all(&directory.path().join("chats"));
    assert_eq!(chats.len(), 1);
    assert!(
        matches!(&chats[0].entries[0], nocterm_ai::thread::Entry::User(blocks) if matches!(&blocks[0], acp::ContentBlock::Text(text) if text.text == "latest dormant document"))
    );
}

#[gpui_kit::test]
fn shutdown_is_durable_even_if_its_task_is_dropped_and_old_writes_are_pending(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| {
        thread
            .state
            .push_user(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "original content",
            ))]);
        thread.name = Some("old snapshot".into());
        thread.save(cx);
    });
    thread.update(cx, |thread, _| {
        thread.name = Some("latest unsaved document".into())
    });
    // Native GPUI may drop this cleanup task after its 200ms grace period.
    // Durability must already hold when the synchronous quit callback returns.
    cx.update(|cx| drop(Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))));
    let read = || nocterm_ai::history::load_all(&f._directory.path().join("chats"));
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
