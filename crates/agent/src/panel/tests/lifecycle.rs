use super::*;

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
            thread.read(cx).composer.attachments,
            vec![Attachment::Terminal(f.terminal)]
        );
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session.clone(),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("answer"),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx|{let thread=f.panel.read(cx).current().unwrap();assert!(matches!(thread.read(cx).state.entries.last(),Some(nocterm_ai::thread::Entry::Agent(value))if value=="answer"));assert!(thread.read(cx).context_bytes>0);assert_eq!(thread.read(cx).tool_bytes,0);let requests=f.commands.prompts.lock().unwrap();assert_eq!(requests.len(),1);assert_eq!(requests[0].prompt.len(),2);});
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!f.workspace.read(cx).right_panel_is_open());
        assert!(window.try_find("toggle-right-panel").is_none());
        assert!(f.panel.read(cx).threads.is_empty());
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
    let f = fixture(cx);
    let started = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    cx.update(|cx| {
        crate::init(
            AgentServices {
                terminal_auth: None,
                private_dirs: Vec::new(),
                shared_dirs: Vec::new(),
                connector: Arc::new(DelayedConnector {
                    started: started.clone(),
                    cancelled: cancelled.clone(),
                }),
                bridge: f.bridge.clone(),
                state_file: f._directory.path().join("state.toml"),
                chats_dir: f._directory.path().join("chats"),
                codex_home: None,
                workdir: f._directory.path().join("pending"),
            },
            &nocterm_ui::UiReady::installed(cx).unwrap(),
            cx,
        )
    });
    new_chat(&f, cx);
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(cancelled.load(Ordering::SeqCst), 0);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.enabled = false)
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
    new_chat(&f, cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
            settings.agents.entry("codex".into()).or_default().command = Some("other-agent".into())
        })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    new_chat(&f, cx);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
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
        crate::runtime::Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.toggle_favorite("codex", "model", "a", cx);
            runtime.toggle_favorite("codex", "model", "b", cx);
            runtime.toggle_favorite("codex", "model", "a", cx);
        })
    });
    cx.run_until_parked();
    let state =
        nocterm_ai::favorites::AgentStateFile::load(&f._directory.path().join("agents.toml"))
            .unwrap();
    assert!(!state.contains("codex", "model", "a"));
    assert!(state.contains("codex", "model", "b"));
}
#[gpui_kit::test]
async fn master_switch_clears_all_windows_and_reenable_is_lazy(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let (handle, workspace, panel) = cx.update(|cx| {
        let mut panel = None;
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    let owner = cx.entity().downgrade();
                    let view = cx.new(|cx| AgentPanel::new(owner, window, cx));
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
    cx.update(|cx| cx.update_setting::<nocterm_ai::AiSettings>(|s| s.enabled = false))
        .await
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(f.panel.read(cx).threads.is_empty());
        assert!(panel.read(cx).threads.is_empty());
        assert!(!f.workspace.read(cx).right_panel_is_open());
        assert!(!workspace.read(cx).right_panel_is_open());
    });
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    assert_eq!(f.bridge.revoked.lock().unwrap().len(), 1);
    cx.update(|cx| cx.update_setting::<nocterm_ai::AiSettings>(|s| s.enabled = true))
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.toggle_right_panel(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("agent-history-empty").is_some());
        assert!(window.try_find("toggle-right-panel").is_some());
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
    let request = serde_json::from_value(serde_json::json!({
        "sessionId": session, "toolCall": {"toolCallId":"call", "title":"Read"},
        "options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]
    }))
    .unwrap();
    f.events
        .try_send(AgentEvent::Permission {
            request,
            respond: tx,
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(thread.read(cx).permissions.len(), 1);
        thread.update(cx, |thread, cx| thread.stop(cx));
    });
    assert!(matches!(
        futures::executor::block_on(rx).unwrap(),
        acp::RequestPermissionOutcome::Cancelled
    ));
    f.commands
        .pending
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(acp::PromptResponse::new(acp::StopReason::Cancelled))
        .unwrap();
    cx.run_until_parked();
    let late = |text: &str| {
        AgentEvent::Session(acp::SessionNotification::new(
            session.clone(),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new(text),
            ))),
        ))
    };
    f.events.try_send(late("late after response")).unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).generating);
        assert!(!thread.read(cx).ended());
        assert_eq!(thread.read(cx).status, "Stopped");
        assert!(
            !thread
                .read(cx)
                .state
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
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 2);
    f.events.try_send(late("answer")).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(thread.read(cx).session().as_ref(), Some(&session));
        assert!(thread.read(cx).state.entries.iter().any(
            |entry| matches!(entry, nocterm_ai::thread::Entry::Agent(text) if text == "answer")
        ));
    });
}

#[gpui_kit::test]
fn authentication_controls_follow_required_state_and_create_session_after_click(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    f.commands.auth_required.store(true, Ordering::SeqCst);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).auth_required);
        assert!(
            thread.read(cx).registration().is_some(),
            "auth-required must preserve bridge for retry"
        );
        assert!(thread.read(cx).session().is_none());
        window.click("authenticate-sign-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).auth_required);
        assert!(thread.read(cx).session().is_some());
        assert_eq!(thread.read(cx).status, "Ready");
        assert!(window.try_find("authenticate-sign-in").is_none());
        assert!(window.try_find("agent-status").is_none());
    })
    .unwrap();
    assert_eq!(f.commands.authentications.load(Ordering::SeqCst), 1);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 2);
}

#[gpui_kit::test]
fn ready_hides_auth_and_status_while_working_status_precedes_composer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("authenticate-sign-in").is_none());
        assert!(window.try_find("agent-status").is_none());
    })
    .unwrap();
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                thread.send("Question".into(), cx);
            });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let status = window.find("agent-status").bounds();
        let composer = window.find("agent-composer").bounds();
        assert!(status.bottom() <= composer.origin.y);
        assert!(window.try_find("authenticate-sign-in").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn prompt_authentication_failure_keeps_retry_and_existing_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.commands.auth_required.store(true, Ordering::SeqCst);
    f.commands
        .authentication_failure
        .store(true, Ordering::SeqCst);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| { thread.send("Question".into(), cx);
                for status in ["pending", "completed"] {
                    thread.state.apply(acp::SessionUpdate::ToolCall(serde_json::from_value(serde_json::json!({"toolCallId":status,"title":"Before auth required","status":status})).unwrap()));
                } });
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).auth_required);
        assert!(!thread.read(cx).generating);
        let statuses = thread
            .read(cx)
            .state
            .entries
            .iter()
            .filter_map(|entry| {
                if let nocterm_ai::thread::Entry::Tool(call) = entry {
                    Some(call.status)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            statuses,
            vec![acp::ToolCallStatus::Failed, acp::ToolCallStatus::Completed]
        );
        thread.update(cx, |thread, cx| {
            thread.send("Do not submit while auth-required".into(), cx)
        });
        assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
        window.click("authenticate-sign-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).auth_required);
        assert!(!thread.read(cx).authenticating);
        assert_eq!(thread.read(cx).status, "Sign-in failed");
        assert!(thread.read(cx).session().is_some());
        f.commands
            .authentication_failure
            .store(false, Ordering::SeqCst);
        window.click("authenticate-sign-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).auth_required);
        assert!(thread.read(cx).session().is_some());
        assert!(window.try_find("authenticate-sign-in").is_none());
    })
    .unwrap();
    assert_eq!(f.commands.authentications.load(Ordering::SeqCst), 2);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn terminal_authentication_uses_host_callback_and_retries_after_cancel(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.commands.auth_required.store(true, Ordering::SeqCst);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let commands = f.commands.clone();
    cx.update(|cx| {
        crate::runtime::Runtime::global(cx).update(cx, |runtime, _| {
            runtime.services.terminal_auth = true;
        });
        cx.set_global(crate::TerminalAuth(Some(Arc::new(
            move |_, request, _, _| {
                let mut requests = captured.lock().unwrap();
                requests.push(request);
                let (send, receive) = oneshot::channel();
                if requests.len() > 1 {
                    commands.auth_required.store(false, Ordering::SeqCst);
                    send.send(Ok(())).unwrap();
                }
                receive
            },
        ))));
    });
    new_chat(&f, cx);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                thread.info.as_mut().unwrap().auth_methods = vec![acp::AuthMethod::Terminal(
                    acp::AuthMethodTerminal::new("setup", "Terminal setup")
                        .args(vec!["--setup".into(), "$(literal_argument)".into()]),
                )];
                cx.notify();
            });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("authenticate-setup", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).auth_required);
        assert!(!thread.read(cx).authenticating);
        assert_eq!(thread.read(cx).status, "Sign-in cancelled");
        window.click("authenticate-setup", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).auth_required);
        assert!(thread.read(cx).session().is_some());
        assert!(window.try_find("authenticate-setup").is_none());
    })
    .unwrap();
    assert_eq!(
        f.commands.authentications.load(Ordering::SeqCst),
        0,
        "terminal methods must not use the agent authenticate RPC"
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].program, "npx");
    assert!(
        requests[0]
            .args
            .ends_with(&["--setup".into(), "$(literal_argument)".into()])
    );
}

#[gpui_kit::test]
fn stopping_and_failure_finalize_only_unfinished_tool_calls(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    for fail in [false, true] {
        cx.update(|cx| {
            let thread = f.panel.read(cx).current().unwrap();
            thread.update(cx, |thread, cx| {
                thread.state.entries = ["pending", "in_progress", "completed", "failed"].into_iter().map(|status| nocterm_ai::thread::Entry::Tool(serde_json::from_value(serde_json::json!({"toolCallId":status,"title":"Tool", "status":status})).unwrap())).collect();
                thread.generating = true;
                thread.accept_updates = true;
                if fail { thread.fail("Adapter error", cx); } else { thread.stop(cx); }
                for (index, entry) in thread.state.entries.iter().enumerate() {
                    let nocterm_ai::thread::Entry::Tool(call) = entry else { panic!() };
                    assert_eq!(call.status, if index == 2 { acp::ToolCallStatus::Completed } else { acp::ToolCallStatus::Failed });
                    assert!(!crate::panel::widgets::entry_is_live(&thread.state.entries, index, thread.generating && thread.accept_updates));
                }
            });
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
fn a_dropped_lease_shuts_its_agent_down(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    thread.update(cx, |thread, _| drop(thread.lease.take().unwrap()));
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn a_permission_no_thread_takes_is_cancelled(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let (respond, answer) = nocterm_ai::PermissionResponder::channel();
    let request = serde_json::from_value(serde_json::json!({
        "sessionId": "unknown-session", "toolCall": {"toolCallId":"call", "title":"Read"},
        "options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]
    }))
    .unwrap();
    f.events
        .try_send(AgentEvent::Permission { request, respond })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        futures::executor::block_on(answer).unwrap(),
        acp::RequestPermissionOutcome::Cancelled
    );
}
