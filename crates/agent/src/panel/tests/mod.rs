use crate::{AgentServices, panel::AgentPanel, runtime::Runtime, thread::Attachment};
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
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
#[derive(Default)]
struct Commands {
    sessions: AtomicU64,
    auth_required: AtomicBool,
    authentications: AtomicUsize,
    authentication_failure: AtomicBool,
    shutdowns: AtomicUsize,
    cancels: AtomicUsize,
    prompts: Mutex<Vec<acp::PromptRequest>>,
    pending: Mutex<Option<oneshot::Sender<acp::PromptResponse>>>,
    /// Each reopened session, and whether it was forked.
    restores: Mutex<Vec<(acp::SessionId, bool)>>,
    /// The MCP servers each new session was given.
    servers: Mutex<Vec<Vec<acp::McpServer>>>,
}
impl AgentCommands for Commands {
    fn new_session(
        &self,
        request: acp::NewSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        self.servers.lock().unwrap().push(request.mcp_servers);
        let id = self.sessions.fetch_add(1, Ordering::SeqCst);
        let auth_required = self.auth_required.load(Ordering::SeqCst);
        async move {
            if auth_required {
                Err(AgentError::AuthRequired("Sign in to continue".into()))
            } else {
                Ok(acp::NewSessionResponse::new(format!("s{id}")))
            }
        }
        .boxed()
    }
    fn restore_session(
        &self,
        request: nocterm_ai::RestoreSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        self.restores
            .lock()
            .unwrap()
            .push((request.session_id.clone(), request.fork));
        let session = if request.fork {
            acp::SessionId::new(format!("fork-of-{}", request.session_id))
        } else {
            request.session_id
        };
        async move { Ok(acp::NewSessionResponse::new(session)) }.boxed()
    }
    fn prompt(
        &self,
        r: acp::PromptRequest,
    ) -> BoxFuture<'static, Result<acp::PromptResponse, AgentError>> {
        self.prompts.lock().unwrap().push(r);
        if self.auth_required.load(Ordering::SeqCst) {
            return async { Err(AgentError::AuthRequired("Sign in to continue".into())) }.boxed();
        }
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
        self.authentications.fetch_add(1, Ordering::SeqCst);
        let failed = self.authentication_failure.load(Ordering::SeqCst);
        if !failed {
            self.auth_required.store(false, Ordering::SeqCst);
        }
        async move {
            if failed {
                Err(AgentError::Io("Sign-in failed".into()))
            } else {
                Ok(())
            }
        }
        .boxed()
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
                    auth_methods: vec![acp::AuthMethod::Agent(acp::AuthMethodAgent::new(
                        "sign-in", "Account",
                    ))],
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
    /// Tool calls as the terminal bridge delivers them.
    calls: async_channel::Sender<BridgeCall>,
    _directory: tempfile::TempDir,
}
fn fixture(cx: &mut TestAppContext) -> Fixture {
    fixture_with_width(cx, 26.)
}
fn fixture_with_width(cx: &mut TestAppContext, width: f32) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let (commands, bridge, connector, events, sender) = {
        let commands = Arc::new(Commands::default());
        let (events, rx) = async_channel::bounded(256);
        let (sender, calls) = async_channel::bounded(16);
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
        (commands, bridge, connector, events, sender)
    };
    let access = Rc::new(Access {
        sent: Default::default(),
        profile: Default::default(),
    });
    let (handle, workspace, panel, terminal) = cx.update(|cx| {
        gpui_kit::init(cx);
        let mut tokens = nocterm_ui::DesignTokens::builtin();
        tokens.layout.agent_panel_width = width;
        nocterm_ui::init(
            tokens,
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init(
            AgentServices {
                terminal_auth: None,
                private_dirs: Vec::new(),
                shared_dirs: Vec::new(),
                connector: connector.clone(),
                bridge: bridge.clone(),
                state_file: directory.path().join("agents.toml"),
                chats_dir: directory.path().join("chats"),
                codex_home: Some(directory.path().join("codex")),
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
        calls: sender,
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
fn exchange(f: &Fixture, text: &str, answer: &str, cx: &mut TestAppContext) -> acp::SessionId {
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send(text.into(), cx));
        thread.read(cx).session.clone().unwrap()
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session.clone(),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new(answer),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish
        .send(acp::PromptResponse::new(acp::StopReason::EndTurn))
        .unwrap();
    cx.run_until_parked();
    session
}

struct Directory(std::cell::RefCell<Vec<nocterm_workspace::ConnectionSummary>>);
impl nocterm_workspace::ConnectionDirectory for Directory {
    fn connections(&self, _: &App) -> Vec<nocterm_workspace::ConnectionSummary> {
        self.0.borrow().clone()
    }
    fn open(
        &self,
        _: &str,
        _: &gpui_kit::WeakEntity<Workspace>,
        _: &mut Window,
        _: &mut App,
    ) -> bool {
        false
    }
    fn open_background(
        &self,
        _: &str,
        _: &gpui_kit::WeakEntity<Workspace>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<gpui_kit::EntityId> {
        None
    }
}

mod approvals;
mod composer;
mod history;
mod lifecycle;
mod routing;
mod servers;
