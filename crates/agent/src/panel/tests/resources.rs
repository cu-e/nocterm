use super::*;
use std::time::Duration;

fn draft(f: &Fixture, cx: &mut TestAppContext) -> Entity<crate::thread::AgentThread> {
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx));
        f.panel.read(cx).current().unwrap()
    })
    .unwrap()
}
fn tick(cx: &mut TestAppContext, seconds: u64) {
    cx.background_executor
        .advance_clock(Duration::from_secs(seconds));
    cx.run_until_parked();
}
#[gpui_kit::test]
fn drafts_history_and_forks_launch_only_after_a_persisted_submission(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let first = draft(&f, cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    first.update(cx, |thread, cx| thread.send("question".into(), cx));
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    complete_active(&f, cx);
    first.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    f.panel
        .update(cx, |panel, cx| panel.open_thread(first.entity_id(), cx));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread(first.entity_id(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(first.read(cx).lease.is_none());
        assert_eq!(first.read(cx).state.entries.len(), 1);
        assert!(f.panel.read(cx).current().unwrap().read(cx).lease.is_none());
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
        assert!(f.access.sent.borrow().is_empty());
    });
}
#[gpui_kit::test]
async fn admission_counts_closing_until_acknowledged_cleanup(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    let first = draft(&f, cx);
    first.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    let second = draft(&f, cx);
    second.update(cx, |thread, cx| thread.send("second".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    let (close, gate) = oneshot::channel();
    *f.commands.close_gate.lock().unwrap() = Some(gate);
    complete_active(&f, cx);
    tick(cx, 1);
    cx.update(|cx| {
        assert!(first.read(cx).lease.is_none());
        assert!(second.read(cx).lease.is_none());
        assert_eq!(second.read(cx).queue.len(), 1);
    });
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    cx.update(|cx| assert!(second.read(cx).generating));
    complete_active(&f, cx);
}
#[gpui_kit::test]
async fn idle_guards_and_paused_queue_release_independently_of_terminal_ownership(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
            settings.sessions.idle_timeout_secs = 2
        })
    })
    .await
    .unwrap();
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let guard = cx.update(|cx| thread.read(cx).hold_operation());
    tick(cx, 5);
    cx.update(|cx| assert!(thread.read(cx).lease.is_some()));
    drop(guard);
    thread.update(cx, |thread, cx| {
        thread.queue_paused = true;
        thread.queue.push(crate::thread::QueuedPrompt::new(
            nocterm_ai::history::SavedPrompt {
                id: 1,
                text: "paused".into(),
                images: Vec::new(),
                attachments: Vec::new(),
            },
            Vec::new(),
        ));
        thread.save(cx);
    });
    tick(cx, 1);
    tick(cx, 3);
    cx.update(|cx| {
        assert!(thread.read(cx).lease.is_none());
        assert_eq!(thread.read(cx).queue.len(), 1);
        assert!(thread.read(cx).queue_paused);
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
        assert!(f.access.sent.borrow().is_empty());
    });
}
#[gpui_kit::test]
fn deletion_during_session_creation_closes_the_orphan_result(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (start, gate) = oneshot::channel();
    *f.commands.session_gate.lock().unwrap() = Some(gate);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let id = f.panel.read(cx).current().unwrap().entity_id();
        f.panel
            .update(cx, |panel, cx| panel.delete_thread(id, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    start.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(
        *f.commands.closed.lock().unwrap(),
        vec![acp::SessionId::new("s0")]
    );
    assert!(f.commands.shutdowns.load(Ordering::SeqCst) >= 1);
    assert!(f.bridge.revoked.lock().unwrap().contains(&1));
}
#[gpui_kit::test]
async fn cleanup_failure_preserves_the_slot_and_queued_document(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    first.update(cx, |thread, _| thread.name = Some("keep".into()));
    f.commands.shutdown_failure.store(true, Ordering::SeqCst);
    first.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    let second = draft(&f, cx);
    second.update(cx, |thread, cx| thread.send("wait".into(), cx));
    cx.run_until_parked();
    tick(cx, 3);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(first.read(cx).status.contains("could not be confirmed"));
        assert_eq!(second.read(cx).queue.len(), 1);
        assert!(second.read(cx).lease.is_none());
    });
}

#[gpui_kit::test]
fn the_same_document_waits_for_its_close_fence_even_with_spare_slots(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    complete_active(&f, cx);
    let (close, gate) = oneshot::channel();
    *f.commands.close_gate.lock().unwrap() = Some(gate);
    thread.update(cx, |thread, cx| {
        thread.release_resources(cx);
        thread.send("resume".into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(thread.read(cx).closing_session);
        assert!(thread.read(cx).lease.is_none());
        assert_eq!(thread.read(cx).queue[0].saved.text, "resume");
    });
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        assert!(!thread.read(cx).closing_session);
        assert!(thread.read(cx).generating);
    });
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(acp::SessionId::new("s0"), false)]
    );
    complete_active(&f, cx);
}
#[gpui_kit::test]
fn dormant_save_failure_keeps_queue_and_prevents_process_launch(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let blocked = f._directory.path().join("not-a-directory");
    std::fs::write(&blocked, b"file").unwrap();
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| runtime.services.chats_dir = blocked)
    });
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| thread.send("retain me".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    cx.update(|cx| {
        assert!(thread.read(cx).persistence_error.is_some());
        assert!(thread.read(cx).queue_paused);
        assert_eq!(thread.read(cx).queue[0].saved.text, "retain me");
        assert_eq!(thread.read(cx).persisted_revision, 0);
    });
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| {
            runtime.services.chats_dir = f._directory.path().join("chats")
        })
    });
    thread.update(cx, |thread, cx| thread.send("next".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(thread.read(cx).persistence_error.is_none());
        assert_eq!(thread.read(cx).queue[0].saved.text, "next");
    });
    complete_active(&f, cx);
    complete_active(&f, cx);
}
#[gpui_kit::test]
fn quit_flushes_documents_that_have_no_live_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, _| {
        thread
            .state
            .push_user(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "unsaved dormant edit",
            ))]);
    });
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx).detach()));
    cx.run_until_parked();
    let chats = nocterm_ai::history::load_all(&f._directory.path().join("chats"));
    assert_eq!(chats.len(), 1);
    assert!(
        matches!(&chats[0].entries[0],nocterm_ai::thread::Entry::User(blocks) if matches!(&blocks[0],acp::ContentBlock::Text(text) if text.text == "unsaved dormant edit"))
    );
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}
#[gpui_kit::test]
async fn admission_is_shared_between_separate_workspace_windows(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    let first = draft(&f, cx);
    first.update(cx, |thread, cx| thread.send("window one".into(), cx));
    cx.run_until_parked();
    let (handle, _workspace, panel) = cx.update(|cx| {
        let mut panel = None;
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    let owner = cx.entity().downgrade();
                    let view = cx.new(|cx| AgentPanel::new(owner, window, cx));
                    workspace.set_right_panel(view.clone(), window, cx);
                    panel = Some(view);
                    workspace
                })
            })
            .unwrap();
        (handle, workspace, panel.unwrap())
    });
    let second = cx
        .update_window(handle, |_, window, cx| {
            panel.update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx));
            panel.read(cx).current().unwrap()
        })
        .unwrap();
    second.update(cx, |thread, cx| thread.send("window two".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| assert_eq!(second.read(cx).queue.len(), 1));
    complete_active(&f, cx);
    tick(cx, 1);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        assert!(first.read(cx).lease.is_none());
        assert!(second.read(cx).generating);
    });
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn warm_idle_limit_evicts_the_oldest_idle_session_and_keeps_documents(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let first = draft(&f, cx);
    first.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    complete_active(&f, cx);
    tick(cx, 1);
    let second = draft(&f, cx);
    second.update(cx, |thread, cx| thread.send("second".into(), cx));
    cx.run_until_parked();
    complete_active(&f, cx);
    tick(cx, 1);
    let third = draft(&f, cx);
    third.update(cx, |thread, cx| thread.send("third".into(), cx));
    cx.run_until_parked();
    complete_active(&f, cx);
    tick(cx, 1);
    cx.update(|cx| {
        assert!(first.read(cx).lease.is_none());
        assert!(second.read(cx).lease.is_some());
        assert!(third.read(cx).lease.is_some());
        assert_eq!(f.panel.read(cx).threads.len(), 3);
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
    });
    assert_eq!(
        *f.commands.closed.lock().unwrap(),
        vec![acp::SessionId::new("s0")]
    );
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    assert!(f.access.sent.borrow().is_empty());
}

#[gpui_kit::test]
async fn starting_sessions_occupy_admission_slots_and_cannot_be_idle_evicted(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
            settings.sessions.max_live = 1;
            settings.sessions.idle_timeout_secs = 1;
        })
    })
    .await
    .unwrap();
    let (start, gate) = oneshot::channel();
    *f.commands.session_gate.lock().unwrap() = Some(gate);
    let first = draft(&f, cx);
    first.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    let second = draft(&f, cx);
    second.update(cx, |thread, cx| thread.send("second".into(), cx));
    cx.run_until_parked();
    tick(cx, 5);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(first.read(cx).connecting_session);
        assert!(first.read(cx).lease.is_some());
        assert!(second.read(cx).lease.is_none());
        assert_eq!(second.read(cx).queue.len(), 1);
    });
    start.send(()).unwrap();
    cx.run_until_parked();
    complete_active(&f, cx);
    tick(cx, 1);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    cx.update(|cx| assert!(second.read(cx).generating));
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn releasing_an_idle_session_finalizes_unfinished_provider_tools(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| thread.send("work".into(), cx));
    cx.run_until_parked();
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    for status in ["pending", "in_progress", "completed"] {
        let call = serde_json::from_value(serde_json::json!({
            "toolCallId": status,
            "title": "Provider tool",
            "status": status
        }))
        .unwrap();
        f.events
            .try_send(AgentEvent::Session(acp::SessionNotification::new(
                session.clone(),
                acp::SessionUpdate::ToolCall(call),
            )))
            .unwrap();
    }
    cx.run_until_parked();
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(thread.read(cx).lease.is_none());
        let tools = thread
            .read(cx)
            .state
            .entries
            .iter()
            .filter_map(|entry| match entry {
                nocterm_ai::thread::Entry::Tool(call) => Some(call.status),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tools,
            vec![
                acp::ToolCallStatus::Failed,
                acp::ToolCallStatus::Failed,
                acp::ToolCallStatus::Completed
            ]
        );
    });
}

#[gpui_kit::test]
fn quit_orders_queued_atomic_writes_before_the_latest_document_snapshot(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| {
        thread
            .state
            .push_user(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "saved message",
            ))]);
        thread.name = Some("old snapshot".into());
        thread.save(cx);
    });
    // The old snapshot has been queued but its writer has not completed.
    thread.update(cx, |thread, _| thread.name = Some("latest document".into()));
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.shutdown(cx).detach()));
    cx.run_until_parked();
    let chats = nocterm_ai::history::load_all(&f._directory.path().join("chats"));
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].name.as_deref(), Some("latest document"));
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn releasing_acp_while_generating_does_not_interrupt_the_live_shell(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| thread.send("observe shell".into(), cx));
    cx.run_until_parked();
    let (registration_id, terminal_id) = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            (
                thread.registration().as_ref().unwrap().id,
                thread.resolved(cx)[0].0.clone(),
            )
        })
    });
    let (respond, _response) = oneshot::channel();
    f.calls
        .try_send(BridgeCall {
            registration_id,
            call: nocterm_ai::TerminalCall::RunCommand(nocterm_ai::RunCommand {
                terminal_id,
                command: "long-running-command".into(),
                timeout_ms: None,
                idle_ms: None,
            }),
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, cx| thread.approve_tool(0, true, false, cx));
    cx.run_until_parked();
    let command_owned = f.access.lease.borrow().as_ref().unwrap().clone();
    assert!(command_owned.load(Ordering::Acquire));
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert!(
        command_owned.load(Ordering::Acquire),
        "releasing ACP cancelled the live-shell lease"
    );
    assert_eq!(*f.access.sent.borrow(), vec!["long-running-command"]);
    cx.update(|cx| assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1));
}

#[gpui_kit::test]
fn manual_release_while_generating_preserves_live_command_ownership(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    f.access.at_prompt.set(false);
    let (close, gate) = oneshot::channel();
    *f.commands.close_gate.lock().unwrap() = Some(gate);
    thread.update(cx, |thread, cx| {
        thread.send("work".into(), cx);
        let terminal_id = thread.resolved(cx)[0].0.clone();
        let (respond, _response) = oneshot::channel();
        thread.handle_tool(
            BridgeCall {
                registration_id: thread.registration().as_ref().unwrap().id,
                call: nocterm_ai::TerminalCall::RunCommand(nocterm_ai::RunCommand {
                    terminal_id,
                    command: "long-running-command".into(),
                    timeout_ms: None,
                    idle_ms: None,
                }),
                respond,
            },
            cx,
        );
        assert!(thread.generating);
        // The default write policy asks first; permit this one command.
        if !thread.tools.is_empty() {
            thread.approve_tool(0, true, false, cx);
        }
        assert!(
            f.access
                .lease
                .borrow()
                .as_ref()
                .unwrap()
                .load(Ordering::Acquire)
        );
        thread.release_resources(cx);
        assert!(
            f.access
                .lease
                .borrow()
                .as_ref()
                .unwrap()
                .load(Ordering::Acquire)
        );
        assert!(thread.lease.is_none());
    });
    cx.run_until_parked();
    tick(cx, 1);
    assert!(
        f.access
            .lease
            .borrow()
            .as_ref()
            .unwrap()
            .load(Ordering::Acquire)
    );
    let late = AgentEvent::Session(acp::SessionNotification::new(
        acp::SessionId::new("s0"),
        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
            acp::TextContent::new("late abandoned response"),
        ))),
    ));
    f.connector.event_senders.lock().unwrap()[0]
        .try_send(late)
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(!thread.read(cx).state.entries.iter().any(|entry| matches!(entry, nocterm_ai::thread::Entry::Agent(text) if text == "late abandoned response"))));
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(
        *f.access.sent.borrow(),
        vec!["long-running-command".to_owned()]
    );
    cx.update(|cx| assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1));
}

#[gpui_kit::test]
fn restart_does_not_treat_the_old_documents_save_ack_as_the_new_queues_ack(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let old = draft(&f, cx);
    old.update(cx, |thread, cx| {
        thread
            .state
            .push_user(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "old message",
            ))]);
        thread.name = Some("old document".into());
        for _ in 0..3 {
            thread.save(cx);
        }
    });
    let chats_dir = f._directory.path().join("chats");
    // Stop the scheduler after the atomic disk write, before its foreground acknowledgement.
    for _ in 0..100 {
        if !nocterm_ai::history::load_all(&chats_dir).is_empty() {
            break;
        }
        assert!(
            cx.background_executor.tick(),
            "old snapshot was never written"
        );
    }
    assert_eq!(nocterm_ai::history::load_all(&chats_dir).len(), 1);
    cx.update(|cx| assert_eq!(old.read(cx).persisted_revision, 0));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_ne!(old.entity_id(), replacement.entity_id());
    replacement.update(cx, |thread, cx| thread.send("new unsaved queue".into(), cx));
    // Simulate disk becoming unwritable for new snapshots while the old successful ACK is queued.
    std::fs::rename(&chats_dir, f._directory.path().join("previous-chats")).unwrap();
    std::fs::write(&chats_dir, b"not a directory").unwrap();
    while cx.background_executor.tick() {
        cx.update(|cx| {
            assert_eq!(
                replacement.read(cx).persisted_revision,
                0,
                "old document acknowledged a different document's queue"
            );
        });
    }
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    cx.update(|cx| {
        assert!(replacement.read(cx).persistence_error.is_some());
        assert_eq!(
            replacement.read(cx).queue[0].saved.text,
            "new unsaved queue"
        );
    });
}

#[gpui_kit::test]
fn replacing_the_document_waits_for_the_saved_chats_closing_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let old = draft(&f, cx);
    old.update(cx, |thread, cx| thread.send("first turn".into(), cx));
    cx.run_until_parked();
    complete_active(&f, cx);
    let (close, gate) = oneshot::channel();
    *f.commands.close_gate.lock().unwrap() = Some(gate);
    old.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    let chat_id = cx.update(|cx| old.read(cx).chat_id.clone());
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_ne!(old.entity_id(), replacement.entity_id());
    cx.update(|cx| assert_eq!(replacement.read(cx).chat_id, chat_id));
    drop(old);
    replacement.update(cx, |thread, cx| thread.send("second turn".into(), cx));
    cx.run_until_parked();
    tick(cx, 1);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert!(replacement.read(cx).lease.is_none());
        assert_eq!(replacement.read(cx).queue[0].saved.text, "second turn");
    });
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(acp::SessionId::new("s0"), false)]
    );
    cx.update(|cx| assert!(replacement.read(cx).generating));
    complete_active(&f, cx);
}

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

#[gpui_kit::test]
async fn saved_drafts_do_not_launch_or_keep_an_idle_agent_alive(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
            settings.sessions.idle_timeout_secs = 2
        })
    })
    .await
    .unwrap();
    let thread = draft(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let input = f.panel.read(cx).input.clone();
        input.update(cx, |input, cx| {
            input.set_value("draft before activation", window, cx)
        });
    })
    .unwrap();
    tick(cx, 1);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    let chats = f._directory.path().join("chats");
    assert_eq!(
        nocterm_ai::history::load_all(&chats)[0].draft.as_deref(),
        Some("draft before activation")
    );
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.send(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    complete_active(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let input = f.panel.read(cx).input.clone();
        input.update(cx, |input, cx| {
            input.set_value("draft retained after idle", window, cx)
        });
    })
    .unwrap();
    tick(cx, 1);
    tick(cx, 3);
    cx.update(|cx| {
        assert!(thread.read(cx).lease.is_none());
        assert_eq!(
            thread.read(cx).draft.as_deref(),
            Some("draft retained after idle")
        );
        assert_eq!(thread.read(cx).state.entries.len(), 1);
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
    });
    assert_eq!(
        nocterm_ai::history::load_all(&chats)[0].draft.as_deref(),
        Some("draft retained after idle")
    );
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).input.read(cx).value().as_ref(),
            "draft retained after idle"
        )
    });
    assert!(f.access.sent.borrow().is_empty());
}
