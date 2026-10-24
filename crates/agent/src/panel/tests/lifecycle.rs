use super::*;
use nocterm_ai::model::auth::PermissionOutcome;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::session::{PromptOutcome, SessionNotice, SessionUpdate, StopReason};
use nocterm_ai::model::tools::ToolStatus;
use nocterm_workspace::test_support::{Toggle, toggle_id};

#[gpui_kit::test]
async fn empty_history_new_chat_streaming_context_and_master_off(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-history-empty").is_some());
    })
    .unwrap();
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .session()
            .clone()
            .unwrap()
    });
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(
            thread.read(cx).composer().attachments(),
            vec![Attachment::Terminal(f.terminal)]
        );
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(SessionNotice::new(
            session.clone(),
            SessionUpdate::AgentMessageChunk(Content::Text(nocterm_ai::model::content::Text::new(
                "answer",
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx|{let thread=f.panel.read(cx).current().unwrap();assert!(matches!(thread.read(cx).transcript().entries.last(),Some(nocterm_ai::thread::Entry::Agent(value))if value=="answer"));assert!(thread.read(cx).context_bytes()>0);assert_eq!(thread.read(cx).tool_bytes(),0);let requests=f.commands.prompts.lock().unwrap();assert_eq!(requests.len(),1);assert_eq!(requests[0].content.len(),2);});
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!f.workspace.read(cx).right_panel_is_open());
        assert!(window.try_find(toggle_id(Toggle::SidePanel)).is_none());
        assert!(f.panel.read(cx).chat_documents().len() == 0);
    })
    .unwrap();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    assert!(!f.bridge.revoked.lock().unwrap().is_empty());
}
struct CancelGuard(Arc<AtomicUsize>);
impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct DelayedConnector {
    started: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
}
impl AgentConnector for DelayedConnector {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        let started = self.started.clone();
        let cancelled = self.cancelled.clone();
        async move {
            started.fetch_add(1, Ordering::SeqCst);
            let _guard = CancelGuard(cancelled);
            request.cancellation.cancelled().await;
            Err(AgentError::Io("Startup cancelled".into()))
        }
        .boxed()
    }
}
#[gpui_kit::test]
async fn disabling_ai_cancels_pending_initialization_immediately(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let started = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    install_fixture(
        &mut f,
        |f| {
            let mut services = fixture_services(f);
            services.connector = Arc::new(DelayedConnector {
                started: started.clone(),
                cancelled: cancelled.clone(),
            });
            services.usage_source = Arc::new(nocterm_ai::testing::ScriptedUsageSource::default());
            services.workdir = f._directory.path().join("pending");
            services
        },
        cx,
    );
    new_chat(&f, cx);
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(cancelled.load(Ordering::SeqCst), 0);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cancelled.load(Ordering::SeqCst), 1);
}
#[gpui_kit::test]
async fn launch_changes_apply_to_new_connections_and_disabled_agents_stop_existing_ones(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    f.connector.use_distinct_process_handles();
    new_chat(&f, cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.agents.entry("codex".into()).or_default().command =
                    Some("other-agent".into())
            })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    new_chat(&f, cx);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.agents.get_mut("codex").unwrap().enabled = false
            })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 2);
}
#[gpui_kit::test]
fn favorites_latest_snapshot_is_persisted_in_order(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        let runtime = f.installation.installed.runtime();
        runtime.toggle_favorite("codex", "model", "a", cx);
        runtime.toggle_favorite("codex", "model", "b", cx);
        runtime.toggle_favorite("codex", "model", "a", cx);
    });
    cx.run_until_parked();
    let state = f.repository.state();
    assert!(!state.contains("codex", "model", "a"));
    assert!(state.contains("codex", "model", "b"));
}
#[gpui_kit::test]
async fn master_switch_clears_all_windows_and_reenable_is_lazy(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    new_chat(&f, cx);
    let (handle, workspace, panel) = cx.update(|cx| {
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
                    let source = workspace.general_surface_source(cx);
                    let view = cx.new(|cx| {
                        AgentPanel::new(
                            f._directory
                                .host(owner, f.installation.installed.clone(), source),
                            window,
                            cx,
                        )
                    });
                    workspace.set_right_panel(view.clone(), window, cx);
                    workspace.set_right_panel_available(true, window, cx);
                    workspace.toggle_right_panel(window, cx);
                    panel = Some(view);
                    workspace
                })
            })
            .unwrap();
        (handle, workspace, panel.unwrap())
    });
    cx.update_window(handle, |_, window, cx| {
        panel.update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |s| s.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(f.panel.read(cx).chat_documents().len() == 0);
        assert!(panel.read(cx).chat_documents().len() == 0);
        assert!(!f.workspace.read(cx).right_panel_is_open());
        assert!(!workspace.read(cx).right_panel_is_open());
    });
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    assert_eq!(f.bridge.revoked.lock().unwrap().len(), 1);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |s| s.enabled = true)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.toggle_right_panel(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("agent-history-empty").is_some());
        assert!(window.try_find(toggle_id(Toggle::SidePanel)).is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn stop_cancels_permissions_drops_late_updates_and_keeps_the_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
        thread.read(cx).session().clone().unwrap()
    });
    cx.run_until_parked();
    let (tx, rx) = nocterm_ai::PermissionResponder::channel();
    let request = permission_request(
        session.clone(),
        "call",
        Some("Read"),
        vec![allow_once_choice("allow", "Allow")],
    );
    f.events
        .try_send(AgentEvent::Permission {
            request,
            respond: tx,
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(thread.read(cx).permission_cards().len(), 1);
        thread.update(cx, |thread, cx| thread.stop(cx));
    });
    assert!(matches!(
        futures::executor::block_on(rx).unwrap(),
        PermissionOutcome::Cancelled
    ));
    f.commands
        .pending
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(PromptOutcome::new(StopReason::Cancelled))
        .unwrap();
    cx.run_until_parked();
    let late = |text: &str| {
        AgentEvent::Session(SessionNotice::new(
            session.clone(),
            SessionUpdate::AgentMessageChunk(Content::Text(nocterm_ai::model::content::Text::new(
                text,
            ))),
        ))
    };
    f.events.try_send(late("late after response")).unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).session_generating());
        assert!(!thread.read(cx).ended());
        assert_eq!(thread.read(cx).status(), "Stopped");
        assert!(
            !thread
                .read(cx)
                .transcript()
                .entries
                .iter()
                .any(|entry| matches!(entry, nocterm_ai::thread::Entry::Agent(_)))
        );
        window.render_frame(cx);
        assert!(window.try_find("agent-restart").is_none());
        // The same session takes the next prompt and its updates.
        thread.update(cx, |thread, cx| thread.send("go on".into(), cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 2);
    f.events.try_send(late("answer")).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(thread.read(cx).session().as_ref(), Some(&session));
        assert!(thread.read(cx).transcript().entries.iter().any(
            |entry| matches!(entry, nocterm_ai::thread::Entry::Agent(text) if text == "answer")
        ));
    });
}

#[gpui_kit::test]
fn stopping_and_failure_finalize_only_unfinished_tool_calls(cx: &mut TestAppContext) {
    let f = fixture(cx);
    for fail in [false, true] {
        new_chat(&f, cx);
        let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
        thread.update(cx,|thread,cx|restore_entries(&f,thread,["pending","in_progress","completed","failed"].into_iter().map(|status|nocterm_ai::thread::Entry::Tool(serde_json::from_value(serde_json::json!({"toolCallId":status,"title":"Tool","status":status})).unwrap())).collect(),cx));
        thread.update(cx, |thread, cx| {
            thread.send("Observe tool finalization".into(), cx)
        });
        cx.run_until_parked();
        thread.update(cx, |thread, cx| {
            if fail {
                thread.fail("Adapter error", cx);
            } else {
                thread.stop(cx);
            }
            for (index, entry) in thread.history_entries().iter().enumerate() {
                let nocterm_ai::thread::Entry::Tool(call) = entry else {
                    continue;
                };
                assert_eq!(
                    call.status,
                    if index == 2 {
                        ToolStatus::Succeeded
                    } else {
                        ToolStatus::Failed
                    }
                );
                assert!(!crate::panel::widgets::entry_is_live(
                    thread.history_entries(),
                    index,
                    thread.session_generating() && thread.session_accepts_updates()
                ));
            }
        });
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            for _ in 0..4 {
                window.simulate_next_frame(cx);
            }
            assert_eq!(
                window.simulate_next_frame(cx),
                0,
                "stopped/failed tool text must stop shimmer after layout settles"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn releasing_chat_resources_shuts_its_agent_down(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn a_permission_no_thread_takes_is_cancelled(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let (respond, answer) = nocterm_ai::PermissionResponder::channel();
    let request = permission_request(
        "unknown-session",
        "call",
        Some("Read"),
        vec![allow_once_choice("allow", "Allow")],
    );
    f.events
        .try_send(AgentEvent::Permission { request, respond })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        futures::executor::block_on(answer).unwrap(),
        PermissionOutcome::Cancelled
    );
}

#[gpui_kit::test]
fn new_chat_fork_and_durable_edits_use_supplied_identity_and_time(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    f._directory.metadata.timestamp_is(1000);
    new_chat(&f, cx);
    let original = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update(|cx| {
        assert_eq!(
            original.read(cx).chat_id(),
            "00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(original.read(cx).updated_at(), 1000);
    });
    f._directory.metadata.timestamp_is(2000);
    original.update(cx, |thread, cx| {
        thread.rename(Some("supplied metadata edit".into()), cx)
    });
    cx.update(|cx| assert_eq!(original.read(cx).updated_at(), 2000));
    f._directory.metadata.timestamp_is(3000);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread(original.entity_id(), window, cx)
        });
    })
    .unwrap();
    cx.update(|cx| {
        let fork = f.panel.read(cx).current().unwrap();
        assert_eq!(
            fork.read(cx).chat_id(),
            "00000000-0000-4000-8000-000000000002"
        );
        assert_eq!(fork.read(cx).updated_at(), 3000);
        assert_eq!(original.read(cx).updated_at(), 2000);
    });
}

#[gpui_kit::test]
fn snapshot_capture_and_rejected_queue_budget_do_not_acquire_native_metadata(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    f._directory.metadata.timestamp_is(1234);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.rename(Some("snapshot candidate".into()), cx);
        let calls = f._directory.metadata.requests();
        let id = thread.chat_id().to_owned();
        let updated = thread.updated_at();
        let _ = thread.shared_snapshot(cx);
        let _ = thread.flush_snapshot(cx);
        let _ = thread.capture_pending(cx);
        let _ = thread.capture_pending(cx);
        assert_eq!(f._directory.metadata.requests(), calls);
        let too_large = "x".repeat(nocterm_ai::history::MAX_FILE_BYTES as usize + 1);
        assert!(thread.submit(too_large, None, cx).is_err());
        assert!(thread.queued_prompt_ids().next().is_none());
        assert_eq!(thread.chat_id(), id);
        assert_eq!(thread.updated_at(), updated);
        assert_eq!(f._directory.metadata.requests(), calls);
    });
}

#[gpui_kit::test]
fn chat_membership_preserves_selection_and_retires_removed_observations(cx: &mut TestAppContext) {
    use crate::roster::chat_list::{ChatList, ChatSlot};
    let f = fixture(cx);
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    first.update(cx, |thread, cx| {
        thread.rename(Some("Keep first".into()), cx)
    });
    new_chat(&f, cx);
    let second = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let events = Rc::new(std::cell::Cell::new(0));
    let mut chats = ChatList::default();
    for thread in [&second, &first] {
        let seen = events.clone();
        let observation = cx.update(|cx| cx.observe(thread, move |_, _| seen.set(seen.get() + 1)));
        let slot = ChatSlot::new(thread.clone(), observation);
        if thread.entity_id() == second.entity_id() {
            chats.append(slot);
            chats.select(second.entity_id());
        } else {
            chats.prepend(vec![slot]);
        }
    }
    assert_eq!(chats.current().unwrap().entity_id(), second.entity_id());
    assert_eq!(chats.iter().count(), 2);
    chats.remove_admitted(&[first.entity_id()]);
    cx.run_until_parked();
    events.set(0);
    first.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(
        events.get(),
        0,
        "removed slot must release its subscription"
    );
    second.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(events.get(), 1, "retained slot keeps its own subscription");
    assert_eq!(chats.current().unwrap().entity_id(), second.entity_id());
    chats.remove_admitted(&[second.entity_id()]);
    assert!(chats.current().is_none());
    assert!(chats.is_empty());
}

#[gpui_kit::test]
fn removed_chat_notifications_do_not_redraw_the_surviving_panel(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let removed = cx.update(|cx| f.panel.read(cx).current().unwrap());
    removed.update(cx, |thread, cx| thread.rename(Some("Old chat".into()), cx));
    new_chat(&f, cx);
    let survivor = cx.update(|cx| f.panel.read(cx).current().unwrap().entity_id());
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("delete-thread", removed.entity_id().as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(40));
    cx.run_until_parked();
    let notifications = Rc::new(std::cell::Cell::new(0));
    let seen = notifications.clone();
    let _observation = cx.update(|cx| cx.observe(&f.panel, move |_, _| seen.set(seen.get() + 1)));
    removed.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(40));
    cx.run_until_parked();
    assert_eq!(notifications.get(), 0);
    cx.update(|cx| {
        assert_eq!(f.panel.read(cx).current().unwrap().entity_id(), survivor);
        assert_eq!(f.panel.read(cx).chat_documents().len(), 1);
    });
}
