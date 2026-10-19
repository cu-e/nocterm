use super::*;
use std::time::Duration;

pub(super) fn draft(f: &Fixture, cx: &mut TestAppContext) -> Entity<crate::thread::AgentThread> {
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
    lazy_start(&f, cx);
    let first = draft(&f, cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 0);
    first.update(cx, |thread, cx| thread.send("question".into(), cx));
    assert_eq!(f.connector.connect_count(), 0);
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
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
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        assert!(!first.read(cx).is_leased());
        assert_eq!(first.read(cx).history_rows().len(), 1);
        assert!(!f.panel.read(cx).current().unwrap().read(cx).is_leased());
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
        assert!(f.access.sent().borrow().is_empty());
    });
}
#[gpui_kit::test]
async fn admission_counts_closing_until_acknowledged_cleanup(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    let first = draft(&f, cx);
    first.update(cx, |thread, cx| thread.send("first".into(), cx));
    cx.run_until_parked();
    let second = draft(&f, cx);
    second.update(cx, |thread, cx| thread.send("second".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
    let close = f.script.gate_close();
    complete_active(&f, cx);
    tick(cx, 1);
    cx.update(|cx| {
        assert!(!first.read(cx).is_leased());
        assert!(!second.read(cx).is_leased());
        assert_eq!(second.read(cx).composer().queue().len(), 1);
    });
    assert_eq!(f.script.shutdowns(), 0);
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 2);
    assert_eq!(f.script.shutdowns(), 1);
    cx.update(|cx| assert!(second.read(cx).session_generating()));
    complete_active(&f, cx);
}
#[gpui_kit::test]
async fn idle_guards_and_paused_queue_release_independently_of_terminal_ownership(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.sessions.idle_timeout_secs = 2
            })
    })
    .await
    .unwrap();
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    // Another chat is shown; the shown one keeps its session.
    thread.update(cx, |thread, cx| thread.set_shown(false, cx));
    let guard = cx.update(|cx| thread.read(cx).hold_operation());
    tick(cx, 5);
    cx.update(|cx| assert!(thread.read(cx).is_leased()));
    drop(guard);
    thread.update(cx, |thread, cx| {
        thread.submit("paused".into(), None, cx).unwrap();
        thread.pause_queue(cx);
        thread.save(cx);
    });
    tick(cx, 1);
    tick(cx, 3);
    cx.update(|cx| {
        assert!(!thread.read(cx).is_leased());
        assert_eq!(thread.read(cx).composer().queue().len(), 1);
        assert!(thread.read(cx).composer().paused());
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
        assert!(f.access.sent().borrow().is_empty());
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
    assert_eq!(f.script.closed_sessions(), vec![SessionId::new("s0")]);
    assert!(f.script.shutdowns() >= 1);
    assert!(f.bridge.revoked.lock().unwrap().contains(&1));
}
#[gpui_kit::test]
async fn cleanup_failure_preserves_the_slot_and_queued_document(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    first.update(cx, |thread, cx| thread.rename(Some("keep".into()), cx));
    f.script.fail_shutdown();
    first.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    let second = draft(&f, cx);
    second.update(cx, |thread, cx| thread.send("wait".into(), cx));
    cx.run_until_parked();
    tick(cx, 3);
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        assert!(first.read(cx).status().contains("could not be confirmed"));
        assert_eq!(second.read(cx).composer().queue().len(), 1);
        assert!(!second.read(cx).is_leased());
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
    let close = f.script.gate_close();
    thread.update(cx, |thread, cx| {
        thread.release_resources(cx);
        thread.send("resume".into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        assert!(thread.read(cx).session_closing());
        assert!(!thread.read(cx).is_leased());
        assert_eq!(thread.read(cx).composer().queued_rows()[0].text(), "resume");
    });
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 2);
    cx.update(|cx| {
        assert!(!thread.read(cx).session_closing());
        assert!(thread.read(cx).session_generating());
    });
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(SessionId::new("s0"), false)]
    );
    complete_active(&f, cx);
}
#[gpui_kit::test]
fn dormant_save_failure_keeps_queue_and_prevents_process_launch(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let thread = draft(&f, cx);
    let id = cx.update(|cx| thread.read(cx).chat_id_string());
    f.repository.fail_next(
        nocterm_ai::testing::StorageOperation::Save(id),
        nocterm_ai::storage::AgentStorageError::Read {
            path: f._directory.path().join("chats"),
            source: Arc::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
        },
    );
    thread.update(cx, |thread, cx| thread.send("retain me".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 0);
    cx.update(|cx| {
        assert!(thread.read(cx).document().persistence_error().is_some());
        assert!(thread.read(cx).composer().paused());
        assert_eq!(
            thread.read(cx).composer().queued_rows()[0].text(),
            "retain me"
        );
        assert_eq!(thread.read(cx).document().published_revision(), 0);
    });
    thread.update(cx, |thread, cx| thread.send("next".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        assert!(thread.read(cx).document().persistence_error().is_none());
        assert_eq!(thread.read(cx).composer().queued_rows()[0].text(), "next");
    });
    complete_active(&f, cx);
    complete_active(&f, cx);
}
#[gpui_kit::test]
fn quit_flushes_documents_that_have_no_live_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.set_draft("unsaved dormant edit".into(), cx);
    });
    cx.update(|cx| f.installation.installed.runtime().shutdown(cx).detach());
    cx.run_until_parked();
    let chats = f.repository.chats();
    assert_eq!(chats.len(), 1);
    assert_eq!(
        chats[0].composer().draft.as_deref(),
        Some("unsaved dormant edit")
    );
    assert_eq!(f.connector.connect_count(), 0);
}
#[gpui_kit::test]
async fn admission_is_shared_between_separate_workspace_windows(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.sessions.max_live = 1)
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
                    let mut workspace = Workspace::new(
                        nocterm_workspace::Deps::new(f.installation.layout.clone()),
                        window,
                        cx,
                    );
                    let owner = cx.entity().downgrade();
                    let view = cx.new(|cx| {
                        AgentPanel::new(
                            f._directory.host(owner, f.installation.installed.clone()),
                            window,
                            cx,
                        )
                    });
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
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| assert_eq!(second.read(cx).composer().queue().len(), 1));
    complete_active(&f, cx);
    tick(cx, 1);
    assert_eq!(f.connector.connect_count(), 2);
    cx.update(|cx| {
        assert!(!first.read(cx).is_leased());
        assert!(second.read(cx).session_generating());
    });
    complete_active(&f, cx);
}

#[gpui_kit::test]
async fn warm_idle_limit_evicts_the_oldest_idle_session_and_keeps_documents(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    // The shown chat is not counted.
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.sessions.max_idle = 1)
    })
    .await
    .unwrap();
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
        assert!(!first.read(cx).is_leased());
        assert!(second.read(cx).is_leased());
        assert!(third.read(cx).is_leased());
        assert_eq!(f.panel.read(cx).chat_documents().len(), 3);
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
    });
    assert_eq!(f.script.closed_sessions(), vec![SessionId::new("s0")]);
    assert_eq!(f.script.shutdowns(), 1);
    assert!(f.access.sent().borrow().is_empty());
}

#[gpui_kit::test]
async fn starting_sessions_occupy_admission_slots_and_cannot_be_idle_evicted(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
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
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        assert!(first.read(cx).session_opening());
        assert!(first.read(cx).is_leased());
        assert!(!second.read(cx).is_leased());
        assert_eq!(second.read(cx).composer().queue().len(), 1);
    });
    start.send(()).unwrap();
    cx.run_until_parked();
    complete_active(&f, cx);
    tick(cx, 1);
    assert_eq!(f.connector.connect_count(), 2);
    cx.update(|cx| assert!(second.read(cx).session_generating()));
    complete_active(&f, cx);
}

#[gpui_kit::test]
fn releasing_an_idle_session_finalizes_unfinished_provider_tools(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| thread.send("work".into(), cx));
    cx.run_until_parked();
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    use nocterm_ai::model::tools::ToolStatus;
    for (id, status) in [
        ("pending", ToolStatus::Pending),
        ("in_progress", ToolStatus::Running),
        ("completed", ToolStatus::Succeeded),
    ] {
        let call = super::provider_intersections::provider_tool(id, "Provider tool", status);
        f.events
            .try_send(AgentEvent::Session(SessionNotice {
                session: session.clone(),
                update: SessionUpdate::ToolStarted(call),
            }))
            .unwrap();
    }
    cx.run_until_parked();
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!thread.read(cx).is_leased());
        let tools = thread
            .read(cx)
            .tool_facts()
            .iter()
            .map(|call| call.status)
            .collect::<Vec<_>>();
        assert_eq!(
            tools,
            vec![
                nocterm_ai::model::tools::ToolStatus::Failed,
                nocterm_ai::model::tools::ToolStatus::Failed,
                nocterm_ai::model::tools::ToolStatus::Succeeded
            ]
        );
    });
}

#[gpui_kit::test]
fn quit_orders_queued_atomic_writes_before_the_latest_document_snapshot(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let thread = draft(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.set_draft("saved message".into(), cx);
        thread.rename(Some("old snapshot".into()), cx);
        thread.save(cx);
    });
    // The old snapshot has been queued but its writer has not completed.
    thread.update(cx, |thread, cx| {
        thread.rename(Some("latest document".into()), cx)
    });
    cx.update(|cx| f.installation.installed.runtime().shutdown(cx).detach());
    cx.run_until_parked();
    let chats = f.repository.chats();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].metadata().name.as_deref(), Some("latest document"));
    assert_eq!(f.connector.connect_count(), 0);
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
            arguments: None,
            display_token: None,
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
    thread.update(cx, |thread, cx| {
        thread.approve_tool(thread.tool_cards()[0].token(), true, false, cx)
    });
    cx.run_until_parked();
    let command_owned = f.access.lease().borrow().as_ref().unwrap().clone();
    assert!(command_owned.load(Ordering::Acquire));
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert!(
        command_owned.load(Ordering::Acquire),
        "releasing ACP cancelled the live-shell lease"
    );
    assert_eq!(*f.access.sent().borrow(), vec!["long-running-command"]);
    cx.update(|cx| assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1));
}

#[gpui_kit::test]
fn manual_release_while_generating_preserves_live_command_ownership(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let lease_held = || {
        f.access
            .lease()
            .borrow()
            .as_ref()
            .unwrap()
            .load(Ordering::Acquire)
    };
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    f.access.at_prompt().set(false);
    let close = f.script.gate_close();
    thread.update(cx, |thread, cx| {
        thread.send("work".into(), cx);
        let terminal_id = thread.resolved(cx)[0].0.clone();
        let (respond, _response) = oneshot::channel();
        f.calls
            .try_send(BridgeCall {
                arguments: None,
                display_token: None,
                registration_id: thread.registration().as_ref().unwrap().id,
                call: nocterm_ai::TerminalCall::RunCommand(nocterm_ai::RunCommand {
                    terminal_id,
                    command: "long-running-command".into(),
                    timeout_ms: None,
                    idle_ms: None,
                }),
                respond,
            })
            .unwrap();
    });
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        assert!(thread.session_generating());
        // The default write policy asks first; permit this one command.
        if !thread.tool_cards().is_empty() {
            thread.approve_tool(thread.tool_cards()[0].token(), true, false, cx);
        }
        assert!(lease_held());
        thread.release_resources(cx);
        assert!(lease_held());
        assert!(!thread.is_leased());
    });
    cx.run_until_parked();
    tick(cx, 1);
    assert!(lease_held());
    let late = AgentEvent::Session(SessionNotice {
        session: SessionId::new("s0"),
        update: SessionUpdate::AgentChunk(Content::Text(Text {
            text: "late abandoned response".into(),
            annotations: None,
            extensions: None,
        })),
    });
    f.connector.process_events(0).try_send(late).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            !thread.read(cx).row_facts().iter().any(
                |row| matches!(row, RowFacts::Agent(text) if text == "late abandoned response")
            )
        )
    });
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(
        *f.access.sent().borrow(),
        vec!["long-running-command".to_owned()]
    );
    cx.update(|cx| assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1));
}

#[gpui_kit::test]
fn restart_does_not_treat_the_old_documents_save_ack_as_the_new_queues_ack(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let old = draft(&f, cx);
    old.update(cx, |thread, cx| {
        thread.set_draft("old message".into(), cx);
        thread.rename(Some("old document".into()), cx);
        for _ in 0..3 {
            thread.save(cx);
        }
    });
    // Stop the scheduler after the atomic disk write, before its foreground acknowledgement.
    for _ in 0..100 {
        if !f.repository.chats().is_empty() {
            break;
        }
        assert!(
            cx.background_executor.tick(),
            "old snapshot was never written"
        );
    }
    assert_eq!(f.repository.chats().len(), 1);
    cx.update(|cx| assert_eq!(old.read(cx).document().published_revision(), 0));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_ne!(old.entity_id(), replacement.entity_id());
    replacement.update(cx, |thread, cx| thread.send("new unsaved queue".into(), cx));
    // Reject the replacement's physical publication while the old successful ACK is queued.
    let replacement_id = cx.update(|cx| replacement.read(cx).chat_id_string());
    f.repository.fail_next(
        nocterm_ai::testing::StorageOperation::Save(replacement_id),
        nocterm_ai::storage::AgentStorageError::Read {
            path: f._directory.path().join("chats"),
            source: Arc::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
        },
    );
    while cx.background_executor.tick() {
        cx.update(|cx| {
            assert_eq!(
                replacement.read(cx).document().published_revision(),
                0,
                "old document acknowledged a different document's queue"
            );
        });
    }
    assert_eq!(f.connector.connect_count(), 0);
    cx.update(|cx| {
        assert!(
            replacement
                .read(cx)
                .document()
                .persistence_error()
                .is_some()
        );
        assert_eq!(
            replacement.read(cx).composer().queued_rows()[0].text(),
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
    let close = f.script.gate_close();
    old.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    let chat_id = cx.update(|cx| old.read(cx).chat_id_string());
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.restart(window, cx));
    })
    .unwrap();
    let replacement = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_ne!(old.entity_id(), replacement.entity_id());
    cx.update(|cx| assert_eq!(replacement.read(cx).chat_id_string(), chat_id));
    drop(old);
    replacement.update(cx, |thread, cx| thread.send("second turn".into(), cx));
    cx.run_until_parked();
    tick(cx, 1);
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        assert!(!replacement.read(cx).is_leased());
        assert_eq!(
            replacement.read(cx).composer().queued_rows()[0].text(),
            "second turn"
        );
    });
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 2);
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(SessionId::new("s0"), false)]
    );
    cx.update(|cx| assert!(replacement.read(cx).session_generating()));
    complete_active(&f, cx);
}

#[gpui_kit::test]
async fn saved_drafts_do_not_launch_or_keep_an_idle_agent_alive(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.sessions.idle_timeout_secs = 2
            })
    })
    .await
    .unwrap();
    let thread = draft(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(
            f.panel.read(cx).composer_focus(cx),
            "draft before activation",
            window,
            cx,
        );
    })
    .unwrap();
    tick(cx, 1);
    assert_eq!(f.connector.connect_count(), 0);
    assert_eq!(
        f.repository.chats()[0].composer().draft.as_deref(),
        Some("draft before activation")
    );
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.send(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
    complete_active(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(
            f.panel.read(cx).composer_focus(cx),
            "draft retained after idle",
            window,
            cx,
        );
    })
    .unwrap();
    // Another chat is shown; the shown one keeps its session.
    thread.update(cx, |thread, cx| thread.set_shown(false, cx));
    tick(cx, 1);
    tick(cx, 3);
    cx.update(|cx| {
        assert!(!thread.read(cx).is_leased());
        assert_eq!(
            thread.read(cx).composer().draft().as_deref(),
            Some("draft retained after idle")
        );
        assert_eq!(thread.read(cx).history_rows().len(), 1);
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
    });
    assert_eq!(
        f.repository.chats()[0].composer().draft.as_deref(),
        Some("draft retained after idle")
    );
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "draft retained after idle"
        )
    });
    assert!(f.access.sent().borrow().is_empty());
}

mod durability;

mod cleanup;
