//! Two windows of one installation: each lists every chat, a chat belongs to one of them.
use super::*;

/// Renders `handle` and runs `check` on it.
fn in_window<R>(
    handle: AnyWindowHandle,
    cx: &mut TestAppContext,
    check: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        check(window, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_second_window_lists_the_first_windows_chats_and_opens_an_idle_one_here(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "Why is the disk full?", "The logs grew.", cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let chat = cx.update(|cx| first.read(cx).chat_id().to_owned());
    // The first window moves on, so the chat neither works nor shows there.
    new_chat(&f, cx);

    let (second, panel) = open_window_panel(&f, cx);
    show_history_in(second, cx);
    let row = first.entity_id().as_u64();
    in_window(second, cx, |window, cx| {
        assert_eq!(
            window.find(("history-elsewhere", row)).label(),
            Some("Open in another window: opens here")
        );
        assert!(window.try_find(("delete-thread", row)).is_none());
        assert_eq!(panel.read(cx).chat_documents().len(), 0, "listed, not held");
        window.click(("history-thread", row), cx);
    });
    cx.run_until_parked();

    let moved = cx.update(|cx| panel.read(cx).current().unwrap());
    cx.update(|cx| {
        let thread = moved.read(cx);
        assert_eq!(thread.chat_id(), chat);
        assert_eq!(thread.transcript().entries.len(), 2, "its whole transcript");
        assert!(
            f.panel
                .read(cx)
                .chat_documents()
                .all(|thread| thread.read(cx).chat_id() != chat),
            "the first window no longer holds it"
        );
    });
    show_history(&f, cx);
    in_window(f.handle, cx, |window, _| {
        assert!(
            window
                .try_find(("history-elsewhere", moved.entity_id().as_u64()))
                .is_some(),
            "the first window lists it where it went"
        );
    });
}

#[gpui_kit::test]
fn a_chat_working_in_another_window_opens_in_that_window(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let working = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update(|cx| working.update(cx, |thread, cx| thread.send("Rotate the logs".into(), cx)));
    cx.run_until_parked();
    new_chat(&f, cx);

    let (second, panel) = open_window_panel(&f, cx);
    show_history_in(second, cx);
    let row = working.entity_id().as_u64();
    in_window(second, cx, |window, cx| {
        assert_eq!(
            window.find(("history-elsewhere", row)).label(),
            Some("Working in another window: opens there")
        );
        window.click(("history-thread", row), cx);
    });
    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).chat_documents().len(),
            0,
            "it stays where it works"
        );
        assert_eq!(
            f.panel.read(cx).current().map(|thread| thread.entity_id()),
            Some(working.entity_id()),
            "its window shows it"
        );
    });
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn a_saved_chat_the_first_window_has_not_opened_moves_with_its_transcript(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.run_until_parked();
    let identity = nocterm_ai::metadata::ChatMetadataSource::new_identity(&*f._directory.metadata);
    let mut chat =
        nocterm_ai::history::SavedChat::for_identity("codex".into(), identity.id, identity.created);
    chat.entries
        .push(nocterm_ai::thread::Entry::Agent("Nginx is up.".into()));
    let archived = super::archive::archived(&f, &chat, cx);
    assert!(cx.update(|cx| archived.read(cx).is_archived()));

    let (second, panel) = open_window_panel(&f, cx);
    show_history_in(second, cx);
    in_window(second, cx, |window, cx| {
        window.click(("history-thread", archived.entity_id().as_u64()), cx);
    });
    cx.run_until_parked();

    let moved = cx.update(|cx| panel.read(cx).current().unwrap());
    cx.update(|cx| {
        assert_eq!(moved.read(cx).chat_id(), chat.header.id);
        assert_eq!(
            moved.read(cx).transcript().entries.len(),
            1,
            "loaded where it went"
        );
        assert!(
            f.panel
                .read(cx)
                .chat_documents()
                .all(|thread| thread.entity_id() != archived.entity_id())
        );
    });
}
