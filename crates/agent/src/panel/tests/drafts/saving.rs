//! Unsent text and edits are saved on shutdown and survive reopening.
use super::*;
use crate::panel::tests::restoration::{disk_chat, reopen};
use nocterm_ai::model::commands::AvailableCommand;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::session::SessionUpdate;

#[gpui_kit::test]
fn closing_the_window_saves_a_draft_without_waiting_for_its_timer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    // Exercise window teardown independently of the application's quit hook.
    cx.update(|cx| cx.set_quit_mode(gpui_kit::QuitMode::Explicit));
    type_draft(&f, "window's final text\n界", cx);
    let (id, thread) = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        (
            thread.read(cx).document().chat_id().to_owned(),
            thread.downgrade(),
        )
    });
    let panel = f.panel.downgrade();
    cx.update_window(f.handle, |_, window, _| window.remove_window())
        .unwrap();
    let repository = f.repository;
    // Release the fixture handles inside an effect cycle so GPUI collects
    // the workspace and its owned panel before checking the weak handles.
    cx.update(|_| {
        drop(f.panel);
        drop(f.workspace);
    });
    cx.run_until_parked();
    panel.assert_released();
    thread.assert_released();
    let value = disk_value(&repository, &id);
    assert_eq!(value["draft"], "window's final text\n界");
    assert!(value["entries"].as_array().unwrap().is_empty());
}

#[gpui_kit::test]
async fn disabling_ai_saves_unsent_text_before_clearing_the_panel(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    type_draft(&f, "draft before disabling AI", cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = thread.read_with(cx, |thread, _| thread.document().chat_id().to_owned());
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(f.panel.read(cx).chat_documents().len() == 0);
        assert!(f.panel.read(cx).composer_text(cx).is_empty());
    });
    let value = disk_value(&f.repository, &id);
    assert_eq!(value["draft"], "draft before disabling AI");
    assert!(value["entries"].as_array().unwrap().is_empty());
    // A retained entity must not replace the saved text with its cleared UI state at quit.
    shutdown_runtime(&f.installation.installed, cx);
    assert_eq!(
        thread.read_with(cx, |thread, _| thread.document().chat_id().to_owned()),
        id
    );
    assert_eq!(disk_value(&f.repository, &id), value);
}

fn adopt(f: &mut Fixture, saved: nocterm_ai::history::SavedChat, cx: &mut TestAppContext) {
    reinstall_history(f, vec![saved], cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn shutdown_saves_a_dormant_chat_without_an_agent_connection(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let mut saved = nocterm_ai::history::SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-4000-8000-00000000006b".into(),
        1,
    );
    let id = saved.id.clone();
    saved.draft = Some("saved text".into());
    adopt(&mut f, saved, cx);
    let shutdown = cx.update(|cx| {
        let thread = f.panel.read(cx).chat_documents().next().unwrap().clone();
        assert!(thread.read(cx).dormant());
        assert!(thread.read(cx).session().is_none());
        thread.update(cx, |thread, cx| {
            thread.set_draft("latest dormant text".into(), cx)
        });
        f.installation.installed.runtime().shutdown(cx)
    });
    cx.foreground_executor().block_test(shutdown);
    assert_eq!(
        disk_value(&f.repository, &id)["draft"],
        "latest dormant text"
    );
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn shutdown_saves_edits_after_the_saved_provider_is_no_longer_configured(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let mut saved = nocterm_ai::history::SavedChat::for_identity(
        "removed-provider".into(),
        "00000000-0000-4000-8000-00000000006c".into(),
        1,
    );
    let id = saved.id.clone();
    saved.draft = Some("previous text".into());
    adopt(&mut f, saved, cx);
    let thread = cx.update(|cx| f.panel.read(cx).chat_documents().next().unwrap().clone());
    cx.update(|cx| assert!(!thread.read(cx).has_status_error()));
    // Listing history stays lazy; opening the chat connects, which reports
    // the missing provider.
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(thread.read(cx).has_status_error());
        assert!(thread.read(cx).session().is_none());
    });
    type_draft(&f, "latest text after provider failure", cx);
    shutdown_runtime(&f.installation.installed, cx);
    assert_eq!(
        disk_value(&f.repository, &id)["draft"],
        "latest text after provider failure"
    );
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn unsent_history_survives_reopen_and_slash_only_then_clears_on_disk(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "remember the first disk", "the first disk is full", cx);
    let saved = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.read(cx).snapshot(cx).unwrap()
    });
    let id = saved.id.clone();
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(AgentError::RestoreUnavailable(
            "Original session is gone".into(),
        ));
    let first = reopen(&f, saved, cx);
    let fresh = disk_chat(&f, &id);
    assert!(fresh.pending_history);
    assert_ne!(fresh.session_id.as_deref(), Some("s0"));
    drop(first);
    cx.run_until_parked();

    // Native resume succeeds for the new session, which has not received old messages.
    let second = reopen(&f, fresh, cx);
    cx.update(|cx| assert!(second.read(cx).document().pending_history()));
    second.update(cx, |thread, cx| {
        report_control(
            thread,
            SessionUpdate::AvailableCommandsUpdate(vec![AvailableCommand::new(
                "help",
                "Show help",
            )]),
            cx,
        );
        thread.send("/help".into(), cx);
    });
    cx.run_until_parked();
    complete_active(&f, cx);
    let after_slash = disk_chat(&f, &id);
    assert!(after_slash.pending_history);
    drop(second);
    cx.run_until_parked();

    let third = reopen(&f, after_slash, cx);
    third.update(cx, |thread, cx| thread.send("what was full?".into(), cx));
    cx.run_until_parked();
    {
        let prompts = f.commands.prompts.lock().unwrap();
        let request = prompts.last().unwrap();
        let history = request
            .content
            .iter()
            .filter_map(|block| match block {
                Content::Text(text) if text.text.contains("Saved conversation") => Some(&text.text),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(history.len(), 1);
        assert!(history[0].contains("User: remember the first disk"));
        assert!(history[0].contains("Assistant: the first disk is full"));
    }
    assert!(
        disk_chat(&f, &id).pending_history,
        "delivery has not finished"
    );
    complete_active(&f, cx);
    let delivered = disk_chat(&f, &id);
    assert!(!delivered.pending_history);
    drop(third);
    cx.run_until_parked();

    let fourth = reopen(&f, delivered, cx);
    cx.update(|cx| assert!(!fourth.read(cx).document().pending_history()));
    fourth.update(cx, |thread, cx| thread.send("next question".into(), cx));
    cx.run_until_parked();
    assert!(!f.commands.prompts.lock().unwrap().last().unwrap().content.iter().any(
        |block| matches!(block, Content::Text(text) if text.text.contains("Saved conversation"))
    ));
    complete_active(&f, cx);
}
