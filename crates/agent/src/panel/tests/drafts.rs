//! Unsent composer text belongs to history without becoming a transcript entry.
use super::*;
use std::time::Duration;

fn set_input(f: &Fixture, text: &str, cx: &mut TestAppContext) {
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

fn saved_value(f: &Fixture, id: &str) -> serde_json::Value {
    let text =
        std::fs::read_to_string(f._directory.path().join("chats").join(format!("{id}.json")))
            .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[gpui_kit::test]
fn unsent_chat_survives_switching_and_restores_its_composer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    set_input(&f, "draft\n界", cx);
    new_chat(&f, cx);
    cx.update(|cx| assert_eq!(f.panel.read(cx).threads.len(), 2));
    f.panel
        .update(cx, |panel, cx| panel.open_thread(first.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "draft\n界"
        );
        assert!(first.read(cx).state.entries.is_empty());
        assert_eq!(first.read(cx).title(), "New Agent");
    });
}

#[gpui_kit::test]
fn ordinary_edits_are_persisted_without_sending_or_switching(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "hello", "answer", cx);
    let id = cx.update(|cx| f.panel.read(cx).current().unwrap().read(cx).chat_id.clone());
    set_input(&f, "unsent text\n界", cx);
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let saved = saved_value(&f, &id);
    assert_eq!(saved["draft"], "unsent text\n界");
    assert_eq!(saved["entries"].as_array().unwrap().len(), 2);
}

#[gpui_kit::test]
fn shutdown_flushes_the_latest_draft_before_a_debounce_can_run(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let id = cx.update(|cx| f.panel.read(cx).current().unwrap().read(cx).chat_id.clone());
    set_input(&f, "last keystroke", cx);
    shutdown_runtime(cx);
    assert_eq!(saved_value(&f, &id)["draft"], "last keystroke");
}

#[gpui_kit::test]
fn restored_history_reopens_the_unsent_text_without_inserting_a_message(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let chat = nocterm_ai::history::SavedChat::new("codex".into());
    let mut value = serde_json::to_value(chat).unwrap();
    value["draft"] = serde_json::json!("restored\ntext");
    let chat = serde_json::from_value(value).unwrap();
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| runtime.saved_chats = Some(vec![chat]))
    });
    let panel = cx
        .update_window(f.handle, |_, window, cx| {
            cx.new(|cx| AgentPanel::new(f.workspace.downgrade(), window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| panel.read(cx).threads[0].clone());
    panel.update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).input.read(cx).value().as_ref(),
            "restored\ntext"
        );
        assert!(thread.read(cx).state.entries.is_empty());
    });
}

#[gpui_kit::test]
fn clear_save_and_quit_in_one_update_cannot_resurrect_an_old_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = thread.read_with(cx, |thread, _| thread.chat_id.clone());
    set_input(&f, "old text", cx);
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(saved_value(&f, &id)["draft"], "old text");
    let shutdown = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.set_draft(String::new(), cx);
            thread.save(cx);
        });
        Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx))
    });
    cx.foreground_executor().block_test(shutdown);
    cx.run_until_parked();
    assert!(nocterm_ai::history::load_all(&f._directory.path().join("chats")).is_empty());
    assert!(saved_value(&f, &id)["draft"].is_null());
}

#[gpui_kit::test]
fn clearing_a_draft_is_reversible_for_the_same_chat_id(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let id = cx.update(|cx| f.panel.read(cx).current().unwrap().read(cx).chat_id.clone());
    for text in ["first", "", "second"] {
        set_input(&f, text, cx);
        cx.background_executor.advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        let chats = nocterm_ai::history::load_all(&f._directory.path().join("chats"));
        if text.is_empty() {
            assert!(chats.is_empty());
        } else {
            assert_eq!(chats.len(), 1);
            assert_eq!(chats[0].id, id);
            assert_eq!(saved_value(&f, &id)["draft"], text);
        }
    }
}

#[gpui_kit::test]
fn normal_send_clears_the_draft_but_noop_and_rejection_retain_it(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    set_input(&f, "  ", cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.send(window, cx));
    })
    .unwrap();
    cx.update(|cx| assert_eq!(thread.read(cx).composer.draft.as_deref(), Some("  ")));
    set_input(&f, "kept when offline", cx);
    thread.update(cx, |thread, _| thread.auth_required = true);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.send(window, cx));
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).composer.draft.as_deref(),
            Some("kept when offline")
        )
    });
    thread.update(cx, |thread, _| thread.auth_required = false);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.send(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(thread.read(cx).composer.draft.is_none());
        assert!(f.panel.read(cx).input.read(cx).value().is_empty());
        assert_eq!(thread.read(cx).state.entries.len(), 1);
    });
    let id = thread.read_with(cx, |thread, _| thread.chat_id.clone());
    assert!(saved_value(&f, &id)["draft"].is_null());
}

#[gpui_kit::test]
fn rapid_new_chats_do_not_copy_the_previous_visible_text(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    set_input(&f, "owner A", cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.new_thread("codex".into(), window, cx);
            panel.new_thread("codex".into(), window, cx);
            assert_eq!(panel.threads.len(), 2);
            assert!(panel.current().unwrap().read(cx).composer.draft.is_none());
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(f.panel.read(cx).input.read(cx).value().is_empty());
        assert_eq!(first.read(cx).composer.draft.as_deref(), Some("owner A"));
    });
}

#[gpui_kit::test]
fn user_input_before_deferred_hydration_wins_and_restart_captures_it(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "first chat", "answer", cx);
    set_input(&f, "A draft", cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    new_chat(&f, cx);
    let second = cx.update(|cx| f.panel.read(cx).current().unwrap());
    set_input(&f, "B draft", cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.open_thread(first.entity_id(), cx);
            panel
                .input
                .update(cx, |input, cx| input.set_value("new user text", window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            first.read(cx).composer.draft.as_deref(),
            Some("new user text")
        );
        assert_eq!(second.read(cx).composer.draft.as_deref(), Some("B draft"));
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "new user text"
        );
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.input.update(cx, |input, cx| {
                input.set_value("restart last character", window, cx)
            });
            panel.restart(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "restart last character"
        )
    });
}

#[gpui_kit::test]
fn sending_a_large_draft_counts_its_text_once_and_rejection_keeps_it(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let text = "x".repeat(17 * 1024 * 1024);
    thread.update(cx, |thread, cx| {
        thread.generating = true;
        thread.set_draft(text.clone(), cx);
        assert_eq!(thread.submit(text, None, cx), Ok(true));
        assert!(thread.composer.draft.is_none());
        assert_eq!(thread.composer.queue.len(), 1);
        // The accepted queued payload already uses over half of the budget.
        // A second payload must fail without removing its composer draft.
        let next = "y".repeat(17 * 1024 * 1024);
        thread.set_draft(next.clone(), cx);
        assert!(thread.submit(next, None, cx).is_err());
        assert!(thread.composer.draft.is_some());
        assert_eq!(thread.composer.queue.len(), 1);
    });
}

#[gpui_kit::test]
fn hiding_the_panel_flushes_its_latest_text_without_waiting_for_the_timer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let id = cx.update(|cx| f.panel.read(cx).current().unwrap().read(cx).chat_id.clone());
    set_input(&f, "before hide", cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace
            .update(cx, |workspace, cx| workspace.toggle_right_panel(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(saved_value(&f, &id)["draft"], "before hide");
}

#[gpui_kit::test]
fn a_retained_restarted_model_cannot_overwrite_the_replacement_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "saved request", "answer", cx);
    set_input(&f, "original draft", cx);
    let old = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = old.read_with(cx, |thread, _| thread.chat_id.clone());
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    set_input(&f, "replacement draft", cx);
    // A late save from the retained old entity has the same history id.
    old.update(cx, |thread, cx| thread.persist(cx));
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(saved_value(&f, &id)["draft"], "replacement draft");
    shutdown_runtime(cx);
    assert_eq!(saved_value(&f, &id)["draft"], "replacement draft");
    cx.update(|_| drop(old));
    cx.run_until_parked();
    assert_eq!(saved_value(&f, &id)["draft"], "replacement draft");
}

#[gpui_kit::test]
async fn disabling_ai_keeps_the_transcript_with_the_final_pending_composer_text(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "saved request", "saved answer", cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = thread.read_with(cx, |thread, _| thread.chat_id.clone());
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.input.update(cx, |input, cx| {
                input.set_value("last pending edit", window, cx)
            });
        });
    })
    .unwrap();
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    let value = saved_value(&f, &id);
    assert_eq!(value["draft"], "last pending edit");
    assert_eq!(value["entries"].as_array().unwrap().len(), 2);
    shutdown_runtime(cx);
    assert_eq!(saved_value(&f, &id), value);
    cx.update(|cx| assert!(thread.read(cx).state.entries.is_empty()));
}

fn history_file_count(f: &Fixture) -> usize {
    let path = f._directory.path().join("chats");
    if path.exists() {
        std::fs::read_dir(path).unwrap().map(Result::unwrap).count()
    } else {
        0
    }
}

#[gpui_kit::test]
fn untouched_chats_do_not_leave_history_files_when_replaced(cx: &mut TestAppContext) {
    let f = fixture(cx);
    for _ in 0..4 {
        new_chat(&f, cx);
    }
    assert_eq!(history_file_count(&f), 0);
}

#[gpui_kit::test]
fn untouched_chat_does_not_create_a_history_file_on_quit(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    shutdown_runtime(cx);
    cx.run_until_parked();
    assert_eq!(history_file_count(&f), 0);
}

#[gpui_kit::test]
async fn disabling_ai_with_an_untouched_chat_does_not_create_a_history_file(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(history_file_count(&f), 0);
}
