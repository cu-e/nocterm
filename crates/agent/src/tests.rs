use crate::{AgentServices, panel::AgentPanel, thread::Attachment};
use futures::{FutureExt as _, channel::oneshot, future::BoxFuture};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, TestAppContext,
    Window, WindowOptions, prelude::*,
};
use nocterm_ai::{
    AgentCommands, AgentConnection, AgentConnector, AgentError, AgentEvent, AgentInfo, BridgeCall,
    BridgeRegistration, ConnectRequest, ToolBridge, acp,
};

use nocterm_workspace::{
    Item, ItemEvent, TerminalAccess, TerminalInfo, TerminalStatus, TerminalText, TextRequest,
    Workspace,
};
use std::{
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
#[derive(Default)]
struct Commands {
    sessions: AtomicU64,
    shutdowns: AtomicUsize,
    cancels: AtomicUsize,
    prompts: Mutex<Vec<acp::PromptRequest>>,
    pending: Mutex<Option<oneshot::Sender<acp::PromptResponse>>>,
}
impl AgentCommands for Commands {
    fn new_session(
        &self,
        _: acp::NewSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        let id = self.sessions.fetch_add(1, Ordering::SeqCst);
        async move { Ok(acp::NewSessionResponse::new(format!("s{id}"))) }.boxed()
    }
    fn prompt(
        &self,
        r: acp::PromptRequest,
    ) -> BoxFuture<'static, Result<acp::PromptResponse, AgentError>> {
        self.prompts.lock().unwrap().push(r);
        let (tx, rx) = oneshot::channel();
        *self.pending.lock().unwrap() = Some(tx);
        async move { rx.await.map_err(|_| AgentError::Io("closed".into())) }.boxed()
    }
    fn cancel(&self, _: acp::SessionId) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
    }
    fn set_mode(
        &self,
        _: acp::SetSessionModeRequest,
    ) -> BoxFuture<'static, Result<(), AgentError>> {
        async { Ok(()) }.boxed()
    }
    fn set_config_option(
        &self,
        _: acp::SetSessionConfigOptionRequest,
    ) -> BoxFuture<'static, Result<Vec<acp::SessionConfigOption>, AgentError>> {
        async { Ok(Vec::new()) }.boxed()
    }
    fn authenticate(&self, _: acp::AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>> {
        async { Ok(()) }.boxed()
    }
    fn close_session(&self, _: acp::SessionId) {}
    fn shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
}
struct Connector {
    commands: Arc<Commands>,
    events: async_channel::Receiver<AgentEvent>,
    connects: AtomicUsize,
}
impl AgentConnector for Connector {
    fn connect(
        &self,
        _: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        let commands = self.commands.clone();
        let events = self.events.clone();
        async move {
            Ok(AgentConnection {
                commands,
                events,
                info: AgentInfo {
                    name: "Fake".into(),
                    version: "1".into(),
                    capabilities: Default::default(),
                    auth_methods: Vec::new(),
                },
            })
        }
        .boxed()
    }
}
struct Bridge {
    next: AtomicU64,
    revoked: Arc<Mutex<Vec<u64>>>,
    stops: AtomicUsize,
    calls: async_channel::Receiver<BridgeCall>,
}
impl ToolBridge for Bridge {
    fn register(&self) -> Result<BridgeRegistration, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let revoked = self.revoked.clone();
        Ok(BridgeRegistration::new(
            id,
            "fake".into(),
            "test-token".into(),
            Arc::new(move |id| revoked.lock().unwrap().push(id)),
        ))
    }
    fn calls(&self) -> async_channel::Receiver<BridgeCall> {
        self.calls.clone()
    }
    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Access {
    sent: std::cell::RefCell<Vec<String>>,
    profile: std::cell::RefCell<Option<gpui_kit::SharedString>>,
}
impl TerminalAccess for Access {
    fn info(&self, _: &App) -> Option<TerminalInfo> {
        Some(TerminalInfo {
            title: "Terminal".into(),
            local: true,
            target: None,
            profile: self.profile.borrow().clone(),
            status: TerminalStatus::Connected,
            cwd: None,
            at_prompt: Some(true),
            dirty_input: false,
            alt_screen: false,
            generation: 1,
        })
    }
    fn read(&self, _: TextRequest, _: &App) -> Result<TerminalText, String> {
        Ok(TerminalText {
            text: "output".into(),
            first_line: 1,
            next_line: 1,
            truncated: false,
            alt_screen: false,
        })
    }
    fn send_text(&self, text: &str, _: &mut App) -> Result<(), String> {
        self.sent.borrow_mut().push(text.into());
        Ok(())
    }
    fn run_command(&self, text: &str, cx: &mut App) -> Result<(), String> {
        self.send_text(text, cx)
    }
}
struct FakeTerminal {
    access: Rc<Access>,
    focus: FocusHandle,
}
impl EventEmitter<ItemEvent> for FakeTerminal {}
impl Focusable for FakeTerminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for FakeTerminal {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div().size_full().track_focus(&self.focus)
    }
}
impl Item for FakeTerminal {
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        "Terminal".into()
    }
    fn terminal_access(&self) -> Option<Rc<dyn TerminalAccess>> {
        Some(self.access.clone())
    }
}
struct Fixture {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    panel: Entity<AgentPanel>,
    commands: Arc<Commands>,
    connector: Arc<Connector>,
    bridge: Arc<Bridge>,
    events: async_channel::Sender<AgentEvent>,
    access: Rc<Access>,
    terminal: gpui_kit::EntityId,
    _directory: tempfile::TempDir,
}
fn fixture(cx: &mut TestAppContext) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let (commands, bridge, connector, events) = {
        let commands = Arc::new(Commands::default());
        let (events, rx) = async_channel::bounded(256);
        let (_, calls) = async_channel::bounded(16);
        let bridge = Arc::new(Bridge {
            next: AtomicU64::new(1),
            revoked: Default::default(),
            stops: AtomicUsize::new(0),
            calls,
        });
        let connector = Arc::new(Connector {
            commands: commands.clone(),
            events: rx,
            connects: AtomicUsize::new(0),
        });
        (commands, bridge, connector, events)
    };
    let access = Rc::new(Access {
        sent: Default::default(),
        profile: Default::default(),
    });
    let (handle, workspace, panel, terminal) = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init(
            AgentServices {
                connector: connector.clone(),
                bridge: bridge.clone(),
                state_file: directory.path().join("agents.toml"),
                workdir: directory.path().join("work"),
            },
            cx,
        );
        let mut panel = None;
        let mut terminal = None;
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    let item = cx.new(|cx| FakeTerminal {
                        access: access.clone(),
                        focus: cx.focus_handle(),
                    });
                    terminal = Some(item.entity_id());
                    workspace.add_item(item, window, cx);
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
        (handle, workspace, panel.unwrap(), terminal.unwrap())
    });
    Fixture {
        handle,
        workspace,
        panel,
        commands,
        connector,
        bridge,
        events,
        access,
        terminal,
        _directory: directory,
    }
}
fn new_chat(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.handle, |_, window, cx| {
        fixture
            .panel
            .update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
async fn empty_history_new_chat_streaming_context_and_master_off(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-empty-new").is_some());
    })
    .unwrap();
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .session
            .clone()
            .unwrap()
    });
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(
            thread.read(cx).attachments,
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
    cx.update(|cx| nocterm_ui::update_settings(cx, |settings| settings.ai.enabled = false))
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
#[gpui_kit::test]
fn shared_connection_approvals_recheck_detachment_and_stop_blocks_late_chunks(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    new_chat(&f, cx);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    let (rx, session) = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        let (tx, rx) = oneshot::channel();
        let registration = thread.read(cx).registration.as_ref().unwrap().id;
        let id = thread.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone());
        thread.update(cx, |thread, cx| {
            thread.handle_tool(
                BridgeCall {
                    registration_id: registration,
                    call: nocterm_ai::TerminalCall::SendInput(nocterm_ai::SendInput {
                        terminal_id: id,
                        text: "danger".into(),
                        press_enter: true,
                    }),
                    respond: tx,
                },
                cx,
            )
        });
        assert_eq!(thread.read(cx).tools.len(), 1);
        thread.update(cx, |thread, _| thread.attachments.clear());
        thread.update(cx, |thread, cx| thread.approve_tool(0, true, true, cx));
        (rx, thread.read(cx).session.clone().unwrap())
    });
    assert!(f.access.sent.borrow().is_empty());
    assert!(futures::executor::block_on(rx).unwrap().is_err());
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            thread.send("hello".into(), cx);
            thread.stop(cx);
            thread.send("blocked".into(), cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("late"),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            !thread.read(cx).state.entries.iter().any(
                |entry| matches!(entry,nocterm_ai::thread::Entry::Agent(value)if value=="late")
            )
        );
    });
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
        _: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        let started = self.started.clone();
        let cancelled = self.cancelled.clone();
        async move {
            started.fetch_add(1, Ordering::SeqCst);
            let _guard = CancelGuard(cancelled);
            futures::future::pending().await
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
                connector: Arc::new(DelayedConnector {
                    started: started.clone(),
                    cancelled: cancelled.clone(),
                }),
                bridge: f.bridge.clone(),
                state_file: f._directory.path().join("state.toml"),
                workdir: f._directory.path().join("pending"),
            },
            cx,
        )
    });
    new_chat(&f, cx);
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(cancelled.load(Ordering::SeqCst), 0);
    cx.update(|cx| nocterm_ui::update_settings(cx, |settings| settings.ai.enabled = false))
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
        nocterm_ui::update_settings(cx, |settings| {
            settings
                .ai
                .agents
                .entry("codex".into())
                .or_default()
                .command = Some("other-agent".into())
        })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    new_chat(&f, cx);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        nocterm_ui::update_settings(cx, |settings| {
            settings.ai.agents.get_mut("codex").unwrap().enabled = false
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
#[test]
fn implicit_markdown_images_are_disabled_and_config_labels_show_selection() {
    assert_eq!(
        super::safe_markdown("![secret](file:///hidden)<img src='x'>"),
        "[secret](file:///hidden)&lt;img src='x'>"
    );
    let option = acp::SessionConfigOption::select(
        "model",
        "Model",
        "one",
        vec![acp::SessionConfigSelectOption::new("one", "First")],
    );
    assert_eq!(super::config_label(&option), "Model: First");
}
#[gpui_kit::test]
fn image_preparation_runs_once_and_renders_a_cached_small_static_preview(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(800, 800))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let data = png.into_inner();
    let key = (1usize, data.len(), false);
    cx.update(|cx| {
        f.panel.update(cx, |panel, cx| {
            panel.cached_image(key, || data, false, cx);
            assert!(matches!(
                panel.image_cache.get(&key),
                Some(super::CachedImage::Loading(_))
            ));
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        f.panel.update(cx, |panel, cx| {
            let Some(super::CachedImage::Ready(image)) = panel.image_cache.get(&key) else {
                panic!("preview not ready");
            };
            let decoded = image::load_from_memory(&image.bytes).unwrap();
            assert!(decoded.width() <= 640 && decoded.height() <= 640);
            panel.cached_image(key, || panic!("cache hit must not decode again"), false, cx);
        })
    });
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
    cx.update(|cx| nocterm_ui::update_settings(cx, |s| s.ai.enabled = false))
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
    assert_eq!(f.bridge.revoked.lock().unwrap().len(), 2);
    cx.update(|cx| nocterm_ui::update_settings(cx, |s| s.ai.enabled = true))
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.toggle_right_panel(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("agent-empty-new").is_some());
        assert!(window.try_find("toggle-right-panel").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn stop_cancels_permissions_and_rejects_late_updates_after_prompt_response(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
        thread.read(cx).session.clone().unwrap()
    });
    cx.run_until_parked();
    let (tx, rx) = oneshot::channel();
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
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("late after response"),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).generating);
        assert!(thread.read(cx).status.contains("Restart"));
        assert!(
            !thread
                .read(cx)
                .state
                .entries
                .iter()
                .any(|entry| matches!(entry, nocterm_ai::thread::Entry::Agent(_)))
        );
        thread.update(cx, |thread, cx| {
            thread.send("cannot reuse cancelled session".into(), cx)
        });
    });
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
}

struct Directory(std::cell::RefCell<Vec<nocterm_workspace::ConnectionSummary>>);
impl nocterm_workspace::ConnectionDirectory for Directory {
    fn connections(&self, _: &App) -> Vec<nocterm_workspace::ConnectionSummary> {
        self.0.borrow().clone()
    }
    fn open(&self, _: &str, _: &mut Window, _: &mut Context<Workspace>) -> bool {
        false
    }
}

#[gpui_kit::test]
fn live_group_membership_and_revoked_registration_reject_stale_terminal_ids(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    *f.access.profile.borrow_mut() = Some("p1".into());
    let summary: nocterm_workspace::ConnectionSummary = nocterm_workspace::ConnectionSummary {
        id: "p1".into(),
        name: "Server".into(),
        group: Some("prod".into()),
        description: "Production".into(),
        target: serde_json::from_value(serde_json::json!({"host":"example.test", "port":22, "user":"user"})).unwrap(),
    };
    let directory = Rc::new(Directory(std::cell::RefCell::new(vec![summary])));
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(directory.clone())
        })
    });
    new_chat(&f, cx);
    let (registration, id) = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            thread.attachments = vec![Attachment::Group("prod".into())];
            let terminals = thread.resolved(cx);
            assert_eq!(terminals.len(), 1);
            (
                thread.registration.as_ref().unwrap().id,
                terminals[0].0.clone(),
            )
        })
    });
    directory.0.borrow_mut()[0].group = Some("other".into());
    for registration_id in [registration, registration + 100] {
        let (tx, rx) = oneshot::channel();
        cx.update(|cx| {
            let thread = f.panel.read(cx).current().unwrap();
            thread.update(cx, |thread, cx| {
                thread.handle_tool(
                    BridgeCall {
                        registration_id,
                        call: nocterm_ai::TerminalCall::ReadTerminal(nocterm_ai::ReadTerminal {
                            terminal_id: id.clone(),
                            lines: None,
                            since: None,
                        }),
                        respond: tx,
                    },
                    cx,
                )
            });
        });
        assert!(futures::executor::block_on(rx).unwrap().is_err());
    }
    directory.0.borrow_mut()[0].group = Some("prod".into());
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| assert_eq!(thread.resolved(cx)[0].0, id));
    });
}
