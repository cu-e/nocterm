//! Agent chat presentation and session coordination through injected contracts.
//! Command jobs privately assemble their owned runner; snapshots and sign-in waits expose immutable facts.
//! Installed retains the actual runtime, image, layout, settings, metadata and auth collaborators.
//! MessageList, ComposerView, HistoryColumn, PanelLayout and MenuView own controls,
//! disposable presentation transitions and their subscription/task lifetimes.
//! ChatList retains original entity selection and slot observations; SignInCards owns
//! secret controls and prompt tokens. PanelNotices owns errors, approval identity and
//! cancellable redraw work; PanelBindings retains installation subscriptions.
//! Stateless renderers take immutable projections; finite presentation operations return
//! typed results to named completions bound to their original view/session authority.
//!
//! Owns: panel views, private Composer/ChatConfig/ChatDocument owners, AgentThread
//! coordination, private SessionDriver and ToolRuntime owners, and the session-scoped
//! ApprovalInbox for provider requests, bridge cards, card tokens and terminal grants.
//! ThreadNotice owns status/error presentation; ThreadBindings retains the window/subscriptions.
//! Finite operations return named completions and original-authority polling projections.
//! Archive owns lazy hydration generations, typed pending intentions and delivery residency.
//! BackgroundOpen owns the native-opening handoff and its original workspace receipt.
//! Does not own: ACP transport, connection admission, filesystem storage or image decoding.
//! Ports used: AgentConnector, ToolBridge, WorkdirResolver, terminal ports,
//! ConnectionDirectory, ImageInspector and ImageThumbnailer through ImageDeps,
//! and explicit ChatMetadataSource retained with each window collaborator.
//! Ports provided: workspace right panel and TerminalAuthOpener composition callback.
//! Settings section: reads ai; its schema belongs to nocterm-ai.
//! Globals: none.
//! Invariants: approval cards retain their original chat and stable token; every
//! choice revalidates current session, registration and attachment authority.
//! Archive delivery also validates document ownership and its local load generation;
//! a literal pending draft supersedes the saved draft and its timestamp together.
//! Preview cache generations reject stale same-key delivery after clear or chat changes.
//! Terminal operation causes stay typed until tool transcripts and sign-in errors are presented.
//! Configuration replies accept only their current session ticket; candidate prompt admission
//! validates bounded payloads before committing draft, image and queue changes.
//! ChatDocument revisions describe accepted durable transitions, not capture or a
//! content fingerprint. Deferred offers remain pending until shared-writer admission;
//! old publication failures cannot pause a newer desired document.
//! Session operations carry their original ticket, connection, session and registration
//! authority. Tool work retains fixed prompt scope, original executor identities and
//! workspace receipts; cancellation preserves readable job outcomes.
//! Joined reconnect calls retain their original busy guards and dispatch in acceptance
//! order after validating original session, registration, workspace and window authority.
//! Ordering releases after synchronous launch; direct calls after connection retain
//! their existing admission path and long-running commands remain independent.

mod chat_host;
mod image_deps;
mod installation;
pub use installation::Installed;
mod panel;
pub use image_deps::ImageDeps;
use nocterm_agent_runtime as runtime;
mod thread;
pub use runtime::TerminalAuthRequest;

use gpui_kit::{App, WeakEntity, Window, prelude::*};
use nocterm_ai::{AgentConnector, ToolBridge};
use std::{path::PathBuf, sync::Arc};

/// Why a terminal sign-in reported through [`TerminalAuthOpener`] did not succeed.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum TerminalAuthError {
    /// The user closed the sign-in terminal.
    #[error("Authentication canceled.")]
    Canceled,
    /// The sign-in command exited unsuccessfully or its terminal closed otherwise.
    #[error("Authentication command did not complete successfully.")]
    Failed,
    /// The agent's sign-in command cannot be launched safely.
    #[error(transparent)]
    InvalidLaunch(#[from] nocterm_session::LaunchValidationError),
}

/// Runs an agent's sign-in command in a terminal of `workspace`, and reports
/// whether it succeeded.
pub type TerminalAuthOpener = Arc<
    dyn Fn(
            WeakEntity<nocterm_workspace::Workspace>,
            TerminalAuthRequest,
            &mut Window,
            &mut App,
        ) -> futures::channel::oneshot::Receiver<Result<(), TerminalAuthError>>
        + Send
        + Sync,
>;

/// What the host provides to the agents.
pub struct AgentServices {
    pub sandbox_probe: Arc<dyn nocterm_ai::sandbox::SandboxProbe>,
    pub metadata: Arc<dyn nocterm_ai::metadata::ChatMetadataSource>,
    pub connector: Arc<dyn AgentConnector>,
    pub bridge: Arc<dyn ToolBridge>,
    pub storage: runtime::AgentStorage,
    /// The host's account usage capability, without native paths in the view.
    pub usage_source: Arc<dyn nocterm_ai::usage::AccountUsageSource>,
    pub workdir_resolver: Arc<dyn nocterm_ai::workspace::WorkdirResolver>,
    pub workdir: PathBuf,
    pub terminal_auth: Option<TerminalAuthOpener>,
    /// nocterm's own directories (settings, vault, chats), hidden from
    /// isolated agents.
    pub private_dirs: Vec<PathBuf>,
    /// Directories isolated agents must still reach, such as the terminal
    /// tools' socket directory.
    pub shared_dirs: Vec<PathBuf>,
}

/// Installs services without starting an agent or opening a listener.
/// The installation remains usable if shutdown registration is refused.
pub fn init(
    services: AgentServices,
    images: ImageDeps,
    layout: nocterm_ui::LayoutHandle,
    ready: &nocterm_ui::UiReady,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> (Installed, Result<(), nocterm_store::StoreError>) {
    panel::commands::init(cx);
    installation::install(services, images, layout, ready, shutdown, cx)
}

gpui_kit::actions!(
    agent,
    [
        NewThread,
        /// Start a new chat with the agent the last chat was started with.
        NewThreadWithLastAgent,
        ShowHistory,
        StopGeneration,
        ToggleModelPicker,
        AttachImage
    ]
);

/// Installs a window-local panel with an initially empty chat history.
pub fn register(
    workspace: &mut nocterm_workspace::Workspace,
    installed: Installed,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<nocterm_workspace::Workspace>,
) {
    let owner = cx.entity().downgrade();
    let host = chat_host::ChatHost::new(owner, installed);
    let enabled = host
        .settings()
        .setting::<nocterm_ai::AiSettings>(cx)
        .enabled;
    let panel = cx.new(|panel_cx| panel::AgentPanel::new(host, window, panel_cx));
    workspace.set_right_panel(panel, window, cx);
    workspace.set_right_panel_available(enabled, window, cx);
}

mod settings_page;
pub use settings_page::settings_page;
