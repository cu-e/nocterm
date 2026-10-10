//! A queued message can be removed without sending it.
use super::*;

fn queued(thread: &Entity<crate::thread::AgentThread>, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        thread
            .read(cx)
            .composer
            .queue
            .iter()
            .map(|prompt| prompt.saved.text.clone())
            .collect()
    })
}
fn render(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn the_queue_removes_a_message_and_keeps_the_rest_in_order(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("first".into(), cx);
        thread.send("second".into(), cx);
        thread.send("third".into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(queued(&thread, cx), ["first", "second", "third"]);
    let second = cx.update(|cx| thread.read(cx).composer.queue[1].saved.id);

    // The bar removes the next message.
    render(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("queue-remove-first", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(queued(&thread, cx), ["second", "third"]);

    // An open queue removes any message.
    f.panel.update(cx, |panel, cx| {
        panel.queue_open = true;
        cx.notify();
    });
    render(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("queued-remove", second), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(queued(&thread, cx), ["third"]);
    cx.update(|cx| {
        let saved = thread.read(cx).snapshot(cx).unwrap();
        let texts: Vec<_> = saved
            .queue
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect();
        assert_eq!(texts, ["third"]);
    });

    // Nothing removed was sent.
    complete_active(&f, cx);
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[1]
            .prompt
            .iter()
            .any(|block| matches!(block, acp::ContentBlock::Text(text) if text.text == "third"))
    );
}

#[gpui_kit::test]
fn a_message_being_edited_cannot_be_removed(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("queued".into(), cx);
    });
    cx.run_until_parked();
    render(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("queue-edit-first", cx);
        window.render_frame(cx);
        window.click("queue-remove-first", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(f.panel.read(cx).composer.edit.is_some()));
    assert_eq!(queued(&thread, cx), ["queued"]);
}

#[gpui_kit::test]
fn removing_the_only_content_of_a_saved_chat_clears_it_on_disk(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(cx);
    let mut saved = nocterm_ai::history::SavedChat::new("codex".into());
    saved.queue.push(nocterm_ai::history::SavedPrompt {
        id: 4,
        text: "never sent".into(),
        images: Vec::new(),
        attachments: Vec::new(),
    });
    let id = saved.id.clone();
    let dir = cx.update(|cx| Runtime::global(cx).read(cx).services.chats_dir.clone());
    nocterm_ai::history::save(&dir, &saved).unwrap();
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| runtime.saved_chats = Some(vec![saved]))
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| f.panel.read(cx).threads[0].clone());
    thread.update(cx, |thread, cx| thread.remove_queued(4, None, cx));
    cx.run_until_parked();
    assert!(
        nocterm_ai::history::load(&dir, &id)
            .unwrap()
            .queue
            .is_empty()
    );
}
