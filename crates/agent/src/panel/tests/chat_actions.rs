//! The title menu copies the original conversation, including offscreen messages.
use super::*;

#[gpui_kit::test]
fn the_title_menu_copies_the_history_id_and_complete_conversation(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let answer = format!("{}\nlast line", "complete answer ".repeat(500));
    exchange(&f, "my question", &answer, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = cx.update(|cx| thread.read(cx).chat_id().to_owned());
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-chat-actions", cx);
        assert!(window.try_find("popup-menu").is_some(), "first opening");
        window.within("popup-menu").click(0usize, cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(id.as_str())
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.click("agent-chat-actions", cx);
        assert!(window.try_find("popup-menu").is_some(), "second opening");
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let copied = cx.read_from_clipboard().unwrap().text().unwrap();
        assert!(copied.contains("## User\n\nmy question"), "{copied}");
        assert!(copied.contains(&answer));
    });
}

#[gpui_kit::test]
fn the_chat_actions_button_sits_after_a_short_title_and_fits_a_narrow_header(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.rename(Some("Audit".into()), cx));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let title = window.find("agent-chat-title").bounds();
        let actions = window.find("agent-chat-actions").bounds();
        assert!(actions.origin.x >= title.right());
        assert!(actions.origin.x - title.right() <= gpui_kit::px(8.));
        assert!(
            window.find("agent-maximize").bounds().right()
                <= window.find("agent-header").bounds().right()
        );
    })
    .unwrap();
    thread.update(cx, |thread, cx| {
        thread.rename(Some("Very long conversation title ".repeat(50)), cx)
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("agent-chat-actions").visible());
        assert!(
            window.find("agent-maximize").bounds().right()
                <= window.find("agent-header").bounds().right()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_chats_dismisses_the_title_menu_and_copies_the_new_history_id(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "first question", "first answer", cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let first_id = cx.update(|cx| first.read(cx).chat_id().to_owned());
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("agent-chat-actions", cx);
        assert!(window.try_find("popup-menu").is_some());
        f.panel
            .update(cx, |panel, cx| panel.select_thread(first.entity_id(), cx));
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        window.click("agent-chat-actions", cx);
        window.within("popup-menu").click(0usize, cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(first_id.as_str())
        );
    })
    .unwrap();
}
