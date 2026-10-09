//! Draft ownership survives view teardown and temporary queue editing.
use super::*;
use std::{path::Path, time::Duration};

fn type_draft(f: &Fixture, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel
                .input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn disk_value(directory: &Path, id: &str) -> serde_json::Value {
    let bytes = std::fs::read(directory.join("chats").join(format!("{id}.json"))).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[gpui_kit::test]
fn closing_the_window_saves_a_draft_without_waiting_for_its_timer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    // Exercise window teardown independently of the application's quit hook.
    cx.update(|cx| cx.set_quit_mode(gpui_kit::QuitMode::Explicit));
    type_draft(&f, "window's final text\n界", cx);
    let (id, thread) = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        (thread.read(cx).chat_id.clone(), thread.downgrade())
    });
    let panel = f.panel.downgrade();
    cx.update_window(f.handle, |_, window, _| window.remove_window())
        .unwrap();
    let directory = f._directory;
    // Release the fixture handles inside an effect cycle so GPUI collects
    // the workspace and its owned panel before checking the weak handles.
    cx.update(|_| {
        drop(f.panel);
        drop(f.workspace);
    });
    cx.run_until_parked();
    panel.assert_released();
    thread.assert_released();
    let value = disk_value(directory.path(), &id);
    assert_eq!(value["draft"], "window's final text\n界");
    assert!(value["entries"].as_array().unwrap().is_empty());
}

#[gpui_kit::test]
async fn disabling_ai_saves_unsent_text_before_clearing_the_panel(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    type_draft(&f, "draft before disabling AI", cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = thread.read_with(cx, |thread, _| thread.chat_id.clone());
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(f.panel.read(cx).threads.is_empty());
        assert!(f.panel.read(cx).input.read(cx).value().is_empty());
    });
    let value = disk_value(f._directory.path(), &id);
    assert_eq!(value["draft"], "draft before disabling AI");
    assert!(value["entries"].as_array().unwrap().is_empty());
    // A retained entity must not replace the saved text with its cleared UI state at quit.
    shutdown_runtime(cx);
    assert_eq!(thread.read_with(cx, |thread, _| thread.chat_id.clone()), id);
    assert_eq!(disk_value(f._directory.path(), &id), value);
}

fn adopt(f: &Fixture, saved: nocterm_ai::history::SavedChat, cx: &mut TestAppContext) {
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| runtime.saved_chats = Some(vec![saved]));
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn shutdown_saves_a_dormant_chat_without_an_agent_connection(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut saved = nocterm_ai::history::SavedChat::new("codex".into());
    let id = saved.id.clone();
    saved.draft = Some("saved text".into());
    adopt(&f, saved, cx);
    let shutdown = cx.update(|cx| {
        let thread = f.panel.read(cx).threads[0].clone();
        assert!(thread.read(cx).dormant);
        assert!(thread.read(cx).session().is_none());
        thread.update(cx, |thread, cx| {
            thread.set_draft("latest dormant text".into(), cx)
        });
        Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))
    });
    cx.foreground_executor().block_test(shutdown);
    assert_eq!(
        disk_value(f._directory.path(), &id)["draft"],
        "latest dormant text"
    );
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn shutdown_saves_edits_after_the_saved_provider_is_no_longer_configured(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut saved = nocterm_ai::history::SavedChat::new("removed-provider".into());
    let id = saved.id.clone();
    saved.draft = Some("previous text".into());
    adopt(&f, saved, cx);
    let thread = cx.update(|cx| f.panel.read(cx).threads[0].clone());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!thread.read(cx).status_error);
        assert!(thread.read(cx).session().is_none());
    });
    // Viewing history stays lazy; a missing provider is reported when work starts.
    thread.update(cx, |thread, cx| thread.request_activation(cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(thread.read(cx).status_error));
    type_draft(&f, "latest text after provider failure", cx);
    shutdown_runtime(cx);
    assert_eq!(
        disk_value(f._directory.path(), &id)["draft"],
        "latest text after provider failure"
    );
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn queued_message_edit_keeps_the_original_draft_on_disk(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread.send("queued original".into(), cx);
    });
    cx.run_until_parked();
    type_draft(&f, "separate unsent draft", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-edit-first", cx);
    })
    .unwrap();
    type_draft(&f, "replacement being edited", cx);
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let id = thread.read_with(cx, |thread, _| thread.chat_id.clone());
    let before = disk_value(f._directory.path(), &id);
    assert_eq!(before["draft"], "separate unsent draft");
    assert_eq!(before["queue"][0]["text"], "queued original");
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let after = disk_value(f._directory.path(), &id);
    assert_eq!(after["draft"], "separate unsent draft");
    assert_eq!(after["queue"][0]["text"], "replacement being edited");
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "separate unsent draft"
        );
    });
}
