use nocterm_ai::model::auth::AuthMethod;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::controls::ConfigOption;
use nocterm_ai::model::ids::{AuthMethodId, SessionId};
use nocterm_ai::model::session::{
    ExternalToolEndpoint, PromptOutcome, PromptRequest, SessionNotice, SessionOpen, SessionOpened,
    SessionUpdate, SetConfigOption, SetMode, StopReason,
};
mod image_fixtures;
use crate::{AgentServices, panel::AgentPanel, runtime, thread::Attachment};
use builders::{allow_once_choice, opened, permission_request};
use futures::{FutureExt as _, channel::oneshot, future::BoxFuture};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, TestAppContext,
    Window, WindowOptions, prelude::*,
};
use image_fixtures::{image_bytes, image_codec, image_deps, prompt_image};
use nocterm_ai::{
    AgentCommands, AgentConnection, AgentConnector, AgentError, AgentEvent, AgentInfo, BridgeCall,
    BridgeRegistration, ConnectRequest, ToolBridge,
};

use nocterm_workspace::{
    ExpectedSession, Item, ItemEvent, LiveCommands, RemoteReconnect, SignInPrompts,
    TerminalCapabilities, TerminalExecution, TerminalInfo, TerminalInput, TerminalState,
    TerminalStatus, TerminalText, TerminalTextSource, TextRequest, UserPromptToken, Workspace,
};
use std::{
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

fn activate_thread(
    thread: &Entity<crate::thread::AgentThread>,
    installed: &crate::Installed,
    cx: &mut App,
) {
    installed
        .runtime()
        .register_document(crate::thread::client(thread), cx);
    thread.update(cx, |thread, cx| thread.request_activation(cx));
}

fn shutdown_runtime(installed: &crate::Installed, cx: &mut TestAppContext) {
    let task = cx.update(|cx| {
        let coordinator = nocterm_store::ShutdownCoordinator::default();
        let runtime = installed.runtime().clone();
        coordinator
            .register_capture(move |cx| runtime.capture_shutdown(cx))
            .unwrap();
        let work = coordinator.capture_all(cx);
        cx.background_executor().spawn(work)
    });
    cx.foreground_executor().block_test(task);
}
mod builders;
mod config_responses;
mod process_commands;

#[derive(Default)]
struct Commands {
    sessions: AtomicU64,
    new_dirs: Mutex<Vec<std::path::PathBuf>>,
    restore_dirs: Mutex<Vec<std::path::PathBuf>>,
    session_gate: Mutex<Option<oneshot::Receiver<()>>>,
    close_gate: Mutex<Option<oneshot::Receiver<()>>>,
    closed: Mutex<Vec<SessionId>>,
    shutdown_failure: AtomicBool,
    auth_required: AtomicBool,
    authentications: AtomicUsize,
    authentication_failure: AtomicBool,
    shutdowns: AtomicUsize,
    cancels: AtomicUsize,
    prompts: Mutex<Vec<PromptRequest>>,
    pending: Mutex<Option<oneshot::Sender<PromptOutcome>>>,
    /// Each reopened session, and whether it was forked.
    restores: Mutex<Vec<(SessionId, bool)>>,
    restore_errors: Mutex<std::collections::VecDeque<AgentError>>,
    /// The MCP servers each new session was given.
    servers: Mutex<Vec<Vec<ExternalToolEndpoint>>>,
    /// The configuration options every session reports.
    config: config_responses::ConfigResponses,
}
impl AgentCommands for Commands {
    fn new_session(
        &self,
        request: SessionOpen,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        self.new_dirs
            .lock()
            .unwrap()
            .push(request.working_directory.clone());
        self.servers.lock().unwrap().push(request.endpoints);
        let gate = self.session_gate.lock().unwrap().take();
        let id = self.sessions.fetch_add(1, Ordering::SeqCst);
        let auth_required = self.auth_required.load(Ordering::SeqCst);
        let config = self.config.options();
        let modes = self.config.modes();
        async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            if auth_required {
                Err(AgentError::AuthRequired("Sign in to continue".into()))
            } else {
                Ok(opened(format!("s{id}"), config, modes))
            }
        }
        .boxed()
    }
    fn restore_session(
        &self,
        request: nocterm_ai::model::session::RestoreSession,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        self.restore_dirs
            .lock()
            .unwrap()
            .push(request.working_directory.clone());
        self.restores
            .lock()
            .unwrap()
            .push((request.session.clone(), request.fork));
        if let Some(error) = self.restore_errors.lock().unwrap().pop_front() {
            return async move { Err(error) }.boxed();
        }
        let session = if request.fork {
            SessionId::new(format!("fork-of-{}", request.session))
        } else {
            request.session
        };
        let config = self.config.options();
        let modes = self.config.modes();
        async move { Ok(opened(session, config, modes)) }.boxed()
    }
    fn prompt(&self, r: PromptRequest) -> BoxFuture<'static, Result<PromptOutcome, AgentError>> {
        self.prompts.lock().unwrap().push(r);
        if self.auth_required.load(Ordering::SeqCst) {
            return async { Err(AgentError::AuthRequired("Sign in to continue".into())) }.boxed();
        }
        let (tx, rx) = oneshot::channel();
        *self.pending.lock().unwrap() = Some(tx);
        async move { rx.await.map_err(|_| AgentError::Io("closed".into())) }.boxed()
    }
    fn cancel(&self, _: SessionId) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
    }
    fn set_mode(&self, _: SetMode) -> BoxFuture<'static, Result<(), AgentError>> {
        async { Ok(()) }.boxed()
    }
    fn set_config_option(
        &self,
        request: SetConfigOption,
    ) -> BoxFuture<'static, Result<Vec<ConfigOption>, AgentError>> {
        let options = self.config.choose(request);
        async move { Ok(options) }.boxed()
    }
    fn authenticate(&self, _: AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>> {
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
    fn close_session(
        &self,
        session: SessionId,
    ) -> BoxFuture<'static, Result<nocterm_ai::CloseSessionOutcome, AgentError>> {
        self.closed.lock().unwrap().push(session);
        let gate = self.close_gate.lock().unwrap().take();
        async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            Ok(nocterm_ai::CloseSessionOutcome::Closed)
        }
        .boxed()
    }
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        self.shutdown();
        let failed = self.shutdown_failure.load(Ordering::SeqCst);
        async move {
            if failed {
                Err(AgentError::Io("Process tree still alive".into()))
            } else {
                Ok(())
            }
        }
        .boxed()
    }
    fn shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
}
struct Connector {
    distinct_process_handles: AtomicBool,
    commands: Arc<Commands>,
    events: async_channel::Receiver<AgentEvent>,
    connects: AtomicUsize,
    roots: Mutex<
        Vec<(
            std::path::PathBuf,
            Option<nocterm_ai::sandbox::SandboxPolicy>,
        )>,
    >,
    event_senders: Mutex<Vec<async_channel::Sender<AgentEvent>>>,
    info: Mutex<AgentInfo>,
}
impl Connector {
    fn use_distinct_process_handles(&self) {
        self.distinct_process_handles.store(true, Ordering::SeqCst);
    }
    fn report_auth_methods(&self, methods: Vec<AuthMethod>) {
        self.info.lock().unwrap().auth_methods = methods;
    }
    fn support_images(&self) {
        self.info.lock().unwrap().capabilities.image_prompts = true;
    }
}
impl AgentConnector for Connector {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        self.roots
            .lock()
            .unwrap()
            .push((request.working_directory, request.sandbox));
        self.connects.fetch_add(1, Ordering::SeqCst);
        let commands: Arc<dyn AgentCommands> =
            if self.distinct_process_handles.load(Ordering::SeqCst) {
                Arc::new(process_commands::ProcessCommands::new(
                    self.commands.clone(),
                ))
            } else {
                self.commands.clone()
            };
        let info = self.info.lock().unwrap().clone();
        let (sender, events) = async_channel::bounded(256);
        self.event_senders.lock().unwrap().push(sender);
        async move {
            Ok(AgentConnection {
                commands,
                events,
                info,
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
    rejections: async_channel::Receiver<nocterm_ai::BridgeRejection>,
}
impl ToolBridge for Bridge {
    fn register(
        &self,
    ) -> Result<BridgeRegistration, nocterm_ai::connection::BridgeRegistrationError> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let revoked = self.revoked.clone();
        Ok(BridgeRegistration::new(
            id,
            nocterm_ai::BridgeLaunch {
                program: "/fake/nocterm".into(),
                args: vec!["agent-bridge".into()],
                env: vec![("NOCTERM_BRIDGE_TOKEN".into(), "test-token".into())],
            },
            Arc::new(move |id| revoked.lock().unwrap().push(id)),
        ))
    }
    fn calls(&self) -> async_channel::Receiver<BridgeCall> {
        self.calls.clone()
    }
    fn rejections(&self) -> async_channel::Receiver<nocterm_ai::BridgeRejection> {
        self.rejections.clone()
    }
    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}
mod terminal_fixture;
use terminal_fixture::{Access, FakeTerminal, SignIn};

struct HostEnvironment {
    directory: tempfile::TempDir,
    metadata: Arc<nocterm_ai::testing::ScriptedChatMetadata>,
}
impl HostEnvironment {
    fn path(&self) -> &std::path::Path {
        self.directory.path()
    }
    fn host(
        &self,
        workspace: gpui_kit::WeakEntity<Workspace>,
        installed: crate::Installed,
    ) -> crate::chat_host::ChatHost {
        crate::chat_host::ChatHost::new(workspace, installed)
    }
}

struct FixtureInstallation {
    ready: nocterm_ui::UiReady,
    shutdown: nocterm_store::ShutdownCoordinator,
    layout: nocterm_ui::LayoutHandle,
    installed: crate::Installed,
}
struct Fixture {
    installation: FixtureInstallation,
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
    rejections: async_channel::Sender<nocterm_ai::BridgeRejection>,
    _directory: HostEnvironment,
    repository: Arc<nocterm_ai::testing::InMemoryAgentRepository>,
}
fn fixture(cx: &mut TestAppContext) -> Fixture {
    fixture_with_width(cx, 26., usage_source(), workspace_resolver(), image_deps())
}
fn workspace_resolver() -> Arc<nocterm_ai::testing::ScriptedWorkdirResolver> {
    Arc::new(nocterm_ai::testing::ScriptedWorkdirResolver::accept_requests())
}
fn usage_source() -> Arc<nocterm_ai::testing::ScriptedUsageSource> {
    Arc::new(nocterm_ai::testing::ScriptedUsageSource::default())
}
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn fixture_with_width(
    cx: &mut TestAppContext,
    width: f32,
    usage_source: Arc<nocterm_ai::testing::ScriptedUsageSource>,
    workdir_resolver: Arc<nocterm_ai::testing::ScriptedWorkdirResolver>,
    images: crate::ImageDeps,
) -> Fixture {
    let directory = HostEnvironment {
        directory: tempfile::tempdir().unwrap(),
        metadata: Arc::new(nocterm_ai::testing::ScriptedChatMetadata::default()),
    };
    let repository = Arc::new(nocterm_ai::testing::InMemoryAgentRepository::default());
    let (commands, bridge, connector, events, sender, rejection_sender) = {
        let commands = Arc::new(Commands::default());
        let (events, rx) = async_channel::bounded(256);
        let (sender, calls) = async_channel::bounded(16);
        let (rejection_sender, rejections) = async_channel::bounded(16);
        let bridge = Arc::new(Bridge {
            next: AtomicU64::new(1),
            revoked: Default::default(),
            stops: AtomicUsize::new(0),
            calls,
            rejections,
        });
        let connector = Arc::new(Connector {
            distinct_process_handles: AtomicBool::new(false),
            commands: commands.clone(),
            events: rx,
            connects: AtomicUsize::new(0),
            roots: Default::default(),
            event_senders: Default::default(),
            info: Mutex::new(AgentInfo {
                name: "Fake".into(),
                version: "1".into(),
                capabilities: Default::default(),
                auth_methods: vec![AuthMethod::agent("sign-in", "Account")],
            }),
        });
        (
            commands,
            bridge,
            connector,
            events,
            sender,
            rejection_sender,
        )
    };
    let access = Access::new(None, Rc::default());
    let (handle, workspace, panel, terminal, ready, shutdown, layout, installed) =
        cx.update(|cx| {
            gpui_kit::init(cx);
            let mut tokens = nocterm_ui::DesignTokens::builtin();
            tokens.layout.agent_panel.width = width;
            let ui = nocterm_ui::init(
                tokens,
                (nocterm_store::SettingsConfig::in_memory(Default::default())).install(cx),
                cx,
            );
            let shutdown = nocterm_store::ShutdownCoordinator::default();
            let layout = nocterm_ui::LayoutMemory::init(None, &shutdown, cx);
            let (installed, registration) = crate::init(
                AgentServices {
                    catalog: nocterm_agent_providers::catalog(),
                    sandbox_probe: Arc::new(nocterm_ai::testing::ScriptedSandboxProbe::new(
                        nocterm_ai::sandbox::Availability::Unsupported,
                    )),
                    metadata: directory.metadata.clone(),
                    terminal_auth: None,
                    private_dirs: Vec::new(),
                    shared_dirs: Vec::new(),
                    connector: connector.clone(),
                    bridge: bridge.clone(),
                    storage: runtime::AgentStorage::prepared(
                        repository.clone(),
                        repository.clone(),
                        Ok(Default::default()),
                    ),
                    usage_source,
                    workdir_resolver,
                    workdir: directory.path().join("work"),
                },
                images,
                layout.clone(),
                &ui,
                &shutdown,
                cx,
            );
            registration.unwrap();
            let mut panel = None;
            installed
                .runtime()
                .inject_events(connector.events.clone(), cx);
            let mut terminal = None;
            let (handle, workspace) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let mut workspace = Workspace::new(
                            nocterm_workspace::Deps::new(layout.clone()),
                            window,
                            cx,
                        );
                        let item = cx.new(|cx| FakeTerminal::new(access.clone(), cx));
                        terminal = Some(item.entity_id());
                        workspace.add_item(item, window, cx);
                        let owner = cx.entity().downgrade();
                        let view = cx.new(|cx| {
                            AgentPanel::new(directory.host(owner, installed.clone()), window, cx)
                        });
                        workspace.set_right_panel(view.clone(), window, cx);
                        workspace.set_right_panel_available(true, window, cx);
                        workspace.toggle_right_panel(window, cx);
                        panel = Some(view);
                        workspace
                    })
                })
                .unwrap();
            (
                handle,
                workspace,
                panel.unwrap(),
                terminal.unwrap(),
                ui,
                shutdown,
                layout,
                installed,
            )
        });
    Fixture {
        installation: FixtureInstallation {
            ready,
            shutdown,
            layout,
            installed,
        },
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
        rejections: rejection_sender,
        _directory: directory,
        repository,
    }
}
fn fixture_services(f: &Fixture) -> AgentServices {
    AgentServices {
        catalog: nocterm_agent_providers::catalog(),
        sandbox_probe: Arc::new(nocterm_ai::testing::ScriptedSandboxProbe::new(
            nocterm_ai::sandbox::Availability::Unsupported,
        )),
        metadata: f._directory.metadata.clone(),
        connector: f.connector.clone(),
        bridge: f.bridge.clone(),
        storage: runtime::AgentStorage::prepared(
            f.repository.clone(),
            f.repository.clone(),
            Ok(f.repository.state()),
        ),
        usage_source: Arc::new(nocterm_ai::testing::ScriptedUsageSource::default()),
        workdir_resolver: workspace_resolver(),
        workdir: f._directory.path().join("work"),
        terminal_auth: None,
        private_dirs: Vec::new(),
        shared_dirs: Vec::new(),
    }
}
/// Simulates a fresh host through public shutdown, installation and view creation.
fn install_fixture(
    f: &mut Fixture,
    services: impl FnOnce(&Fixture) -> AgentServices,
    cx: &mut TestAppContext,
) {
    shutdown_runtime(&f.installation.installed, cx);
    let services = services(f);
    cx.update(|cx| {
        let (installed, registration) = crate::init(
            services,
            image_deps(),
            f.installation.layout.clone(),
            &f.installation.ready,
            &f.installation.shutdown,
            cx,
        );
        registration.unwrap();
        f.installation.installed = installed;
        {
            let runtime = f.installation.installed.runtime();
            runtime.inject_events(f.connector.events.clone(), cx)
        };
    });
    let panel = cx
        .update_window(f.handle, |_, window, cx| {
            let panel = cx.new(|cx| {
                AgentPanel::new(
                    f._directory
                        .host(f.workspace.downgrade(), f.installation.installed.clone()),
                    window,
                    cx,
                )
            });
            f.workspace.update(cx, |workspace, cx| {
                workspace.set_right_panel(panel.clone(), window, cx)
            });
            panel
        })
        .unwrap();
    f.panel = panel;
}
/// Supplies bounded prepared records to the real take-once catalog owner.
fn reinstall_history(
    f: &mut Fixture,
    chats: Vec<nocterm_ai::history::SavedChat>,
    cx: &mut TestAppContext,
) {
    let history = runtime::PreparedHistory::new(chats).unwrap();
    install_fixture(
        f,
        |f| {
            let mut services = fixture_services(f);
            services.storage = services.storage.with_history(history);
            services
        },
        cx,
    );
}
/// Connects chats on their first message instead of when shown, for tests
/// of what happens before a chat connects.
fn lazy_start(f: &Fixture, cx: &mut TestAppContext) {
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.sessions.warm_start = false)
            .detach()
    });
    cx.run_until_parked();
}
mod panel_actions;
use panel_actions::{new_chat, open_queue, replace_composer_value, replace_textbox, show_history};
fn complete_active(f: &Fixture, cx: &mut TestAppContext) {
    f.commands
        .pending
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(PromptOutcome::new(StopReason::EndTurn))
        .unwrap();
    cx.run_until_parked();
}

fn exchange(f: &Fixture, text: &str, answer: &str, cx: &mut TestAppContext) -> SessionId {
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send(text.into(), cx));
        thread.read(cx).session().clone().unwrap()
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(SessionNotice::new(
            session.clone(),
            SessionUpdate::AgentMessageChunk(Content::Text(nocterm_ai::model::content::Text::new(
                answer,
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish
        .send(PromptOutcome::new(StopReason::EndTurn))
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
mod bridge_outcomes;
mod composer;
mod durable_config;
mod hermes;
mod history;
mod invalidation;
mod labels;
mod lifecycle;
mod navigation;
mod persistence;
mod provider_fixtures;
mod provider_permissions;
mod queue_remove;
mod reconnect;
mod resources;
mod restoration;
mod routing;
mod servers;
mod tool_corpus;
mod tool_input;
mod tool_output;
mod tool_presentation;
mod tool_sections;
mod tool_wrappers;
mod usage_dismissal;
mod warm_start;

mod flow;

mod queue_edit;
mod queue_layout;

mod commands;

mod execution;

mod drafts;

mod keep_alive;
mod recovery;

mod archive;

fn report_control(
    thread: &mut crate::thread::AgentThread,
    update: SessionUpdate,
    cx: &mut Context<crate::thread::AgentThread>,
) {
    let session = thread
        .session()
        .clone()
        .expect("fixture chat has an open agent session");
    thread.session_update(&SessionNotice::new(session, update), cx);
}

fn restored_projection(
    thread: &crate::thread::AgentThread,
    cx: &gpui_kit::App,
) -> nocterm_ai::history::SavedChat {
    let mut chat = thread.snapshot(cx).unwrap_or_else(|| {
        nocterm_ai::history::SavedChat::for_identity(
            thread.document().agent_id().to_owned(),
            thread.chat_id().to_owned(),
            thread.document().updated(),
        )
    });
    if let Some(session) = thread.session() {
        chat.session_id = Some(session.to_string());
        chat.workdir = thread.session_workdir().clone();
    }
    chat
}
fn restore_entries(
    f: &Fixture,
    thread: &mut crate::thread::AgentThread,
    entries: Vec<nocterm_ai::thread::Entry>,
    cx: &mut Context<crate::thread::AgentThread>,
) {
    let mut chat = restored_projection(thread, cx);
    chat.entries = entries;
    chat.times.clear();
    f.repository.seed_chat(&chat).unwrap();
    thread.restore_content(chat);
    thread.preview_config(cx);
    for row in 0..thread.history_entries().len() {
        thread.mark_dirty(row);
    }
    cx.notify();
}
fn restore_pending_history(
    f: &Fixture,
    thread: &mut crate::thread::AgentThread,
    cx: &mut Context<crate::thread::AgentThread>,
) {
    let mut chat = restored_projection(thread, cx);
    chat.pending_history = true;
    f.repository.seed_chat(&chat).unwrap();
    thread.restore_content(chat);
    thread.preview_config(cx);
    cx.notify();
}

fn set_input(f: &Fixture, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        replace_composer_value(f.panel.read(cx).composer_focus(cx), text, window, cx);

        window.render_frame(cx);
    })
    .unwrap();
}
