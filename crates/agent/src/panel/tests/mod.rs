mod image_fixtures;
use crate::thread::test_views::RowFacts;
use crate::{AgentServices, panel::AgentPanel, runtime, thread::Attachment};
use futures::{FutureExt as _, channel::oneshot, future::BoxFuture};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, TestAppContext,
    Window, WindowOptions, prelude::*,
};
use image_fixtures::{attach_image, image_bytes, image_codec, image_deps};
use nocterm_ai::{
    AgentConnector, AgentEvent, AgentInfo, BridgeCall, BridgeRegistration, ToolBridge,
};

use nocterm_ai::model::{
    auth::{AuthFlow, AuthMethod},
    content::{Content, Text},
    controls::{ConfigOption, Modes},
    ids::{AuthMethodId, SessionId},
    notice::{SessionNotice, SessionUpdate},
    session::{
        ConfigurationSelection, EndOfTurn, ExternalToolEndpoint, ModeSelection, PromptOutcome,
        PromptRequest, SessionOpen, SessionOpened,
    },
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
mod config_responses;
mod managed_commands;
mod scripted_failure;
use scripted_failure::RestoreFailure;

#[derive(Default)]
struct Commands {
    sessions: AtomicU64,
    new_dirs: Mutex<Vec<std::path::PathBuf>>,
    restore_dirs: Mutex<Vec<std::path::PathBuf>>,
    session_gate: Mutex<Option<oneshot::Receiver<()>>>,
    auth_required: AtomicBool,
    /// Prompt preparation refuses before the turn is committed.
    prompt_refused: AtomicBool,
    authentications: AtomicUsize,
    authentication_failure: AtomicBool,
    cancels: AtomicUsize,
    prompts: Mutex<Vec<managed_commands::PromptRecord>>,
    pending: Mutex<Option<oneshot::Sender<PromptOutcome>>>,
    /// Each reopened session, and whether it was forked.
    restores: Mutex<Vec<(SessionId, bool)>>,
    restore_errors: Mutex<std::collections::VecDeque<RestoreFailure>>,
    /// The MCP servers each new session was given.
    servers: Mutex<Vec<Vec<ExternalToolEndpoint>>>,
    /// The configuration options every session reports.
    config: config_responses::ConfigResponses,
}
mod connector;
use connector::Connector;
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
use terminal_fixture::{Access, ConnectedAccess, FakeTerminal, SignIn};

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
    retention: nocterm_ai::model::retention::RetentionAuthority,
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
    /// Close and shutdown of every fixture connection.
    script: nocterm_ai::testing::ScriptedCommands,
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
    let retention = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let repository = Arc::new(nocterm_ai::testing::InMemoryAgentRepository::new(
        retention.clone(),
    ));
    let (commands, script, bridge, connector, events, sender, rejection_sender) = {
        let commands = Arc::new(Commands::default());
        let script = nocterm_ai::testing::ScriptedCommands::with_sessions(
            managed_commands::SessionFixture::new(commands.clone()),
        );
        script.complete_closes();
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
        let connector = Arc::new(Connector::new(script.clone(), rx));
        (
            commands,
            script,
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
                    retention: retention.clone(),
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
            installed.runtime().inject_events(connector.events(), cx);
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
            retention,
            ready,
            shutdown,
            layout,
            installed,
        },
        handle,
        workspace,
        panel,
        commands,
        script,
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
        retention: f.installation.retention.clone(),
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
            runtime.inject_events(f.connector.events(), cx)
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
    for chat in &chats {
        f.repository.seed_chat(chat).unwrap();
    }
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
    end_active(f, EndOfTurn::Finished, cx);
}

fn end_active(f: &Fixture, ending: EndOfTurn, cx: &mut TestAppContext) {
    f.commands
        .pending
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(PromptOutcome {
            ending,
            tokens: None,
        })
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
        .try_send(AgentEvent::Session(SessionNotice {
            session: session.clone(),
            update: SessionUpdate::AgentChunk(Content::Text(Text {
                text: answer.into(),
                annotations: None,
                extensions: None,
            })),
        }))
        .unwrap();
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish
        .send(PromptOutcome {
            ending: EndOfTurn::Finished,
            tokens: None,
        })
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
mod provider_intersections;
mod provider_permissions;
mod queue_remove;
mod reconnect;
mod resources;
mod restoration;
mod routing;
mod servers;
mod tool_input;
mod tool_output;
mod tool_presentation;
mod tool_sections;
mod tool_wrappers;
mod usage_dismissal;
mod warm_start;

mod flow;

mod queue_layout;
mod verification;

mod commands;

mod audit;
mod execution;

mod drafts;

mod drafts_intersections;
mod keep_alive;
mod recovery;
mod restoration_intersections;

mod archive;

fn report_control(
    f: &Fixture,
    thread: &mut crate::thread::AgentThread,
    update: SessionUpdate,
    cx: &mut Context<crate::thread::AgentThread>,
) {
    let session = thread
        .session()
        .clone()
        .expect("fixture chat has an open agent session");
    thread.session_update(f.connector.notice(session, update), cx);
}

/// Delivers `entries` into the open chat the way its agent reports them. User rows
/// only come from sent prompts, so they are not accepted here.
fn restore_entries(
    f: &Fixture,
    thread: &mut crate::thread::AgentThread,
    entries: Vec<nocterm_ai::thread::Entry>,
    cx: &mut Context<crate::thread::AgentThread>,
) {
    use nocterm_ai::thread::Entry;
    for entry in entries {
        let update = match entry {
            Entry::Agent(text) => SessionUpdate::AgentChunk(Content::Text(Text {
                text,
                annotations: None,
                extensions: None,
            })),
            Entry::Thought(text) => SessionUpdate::ThoughtChunk(Content::Text(Text {
                text,
                annotations: None,
                extensions: None,
            })),
            Entry::Tool(call) => SessionUpdate::ToolStarted(call),
            Entry::Content(content) => SessionUpdate::AgentChunk(content),
            Entry::User(_) => panic!("user rows come from sent prompts"),
        };
        report_control(f, thread, update, cx);
    }
    cx.notify();
}
fn restore_pending_history(
    _: &Fixture,
    thread: &mut crate::thread::AgentThread,
    cx: &mut Context<crate::thread::AgentThread>,
) {
    thread.mark_history_pending();
    cx.notify();
}
/// Delivers `update` to `session` through the chat's event stream.
fn deliver(f: &Fixture, session: &SessionId, update: SessionUpdate, cx: &mut TestAppContext) {
    f.events
        .try_send(AgentEvent::Session(SessionNotice {
            session: session.clone(),
            update,
        }))
        .unwrap();
    cx.run_until_parked();
}
