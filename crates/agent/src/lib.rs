//! The agent chat panel: views over the chats of `nocterm-agent-chat`.
//! Installed retains the actual runtime, available agents, image, layout, settings, metadata
//! and auth collaborators.
//! MessageList, ComposerView, HistoryColumn, PanelLayout and MenuView own controls,
//! disposable presentation transitions and their subscription/task lifetimes.
//! SignInCards owns secret controls and prompt tokens. PanelNotices owns errors, approval
//! identity and cancellable redraw work; PanelBindings retains installation subscriptions.
//! Stateless renderers take immutable projections; finite presentation operations return
//! typed results to named completions bound to their original view/session authority.
//!
//! Owns: panel views and presentation state, the panel's ChatRoster, WorkspaceSeat (the
//! ChatWorkspace over an original surface and exact ChatKey) and WorkspaceSignIn (the SignInTerminal over the
//! host's TerminalAuthOpener).
//! Does not own: chats, sessions and tools (nocterm-agent-chat), ACP transport, connection
//! admission, filesystem storage or image decoding.
//! Ports used: AgentConnector, ToolBridge, WorkdirResolver, ConnectionDirectory, ChatConnectionOpen,
//! ImageInspector and ImageThumbnailer through ImageDeps, and ChatMetadataSource.
//! Ports provided: the required `agent` system plugin (`plugin::PLUGIN`) with its side panel,
//! and the TerminalAuthOpener composition callback.
//! Settings section: reads ai; its schema belongs to nocterm-ai.
//! Globals: none.
//! Invariants: preview cache generations reject stale same-key delivery after clear or chat
//! changes. Committed outer-surface changes select one scoped roster; async delivery keeps
//! its original owner. Accepted hidden forks finish in that original collection without
//! changing the visible chat. Ordinary terminal operations retain the original surface;
//! explicitly attached saved servers open into that same terminal collection.

mod agent_icon;
mod chat_host;
mod image_deps;
mod installation;
pub use installation::Installed;
mod panel;
pub mod plugin;
pub use image_deps::ImageDeps;
use nocterm_agent_runtime as runtime;
mod workspace_seat;
mod workspace_sign_in;
pub use nocterm_agent_chat::TerminalAuthError;
use nocterm_agent_chat::{chat_deps, roster, sign_in, thread};
pub use runtime::TerminalAuthRequest;

use gpui_kit::{App, WeakEntity, Window, prelude::*};
use nocterm_ai::{AgentConnector, ToolBridge};
use std::{path::PathBuf, sync::Arc};

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
    /// The agents nocterm ships profiles for.
    pub catalog: nocterm_ai::AgentCatalog,
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
///
/// A window whose chat context cannot be bound gets no panel; that is logged, not an error.
pub fn register(
    workspace: &mut nocterm_workspace::Workspace,
    installed: Installed,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<nocterm_workspace::Workspace>,
) -> Result<(), nocterm_workspace::RegistrationError> {
    let owner = cx.entity().downgrade();
    let source = workspace.context_source(cx);
    let host = match chat_host::ChatHost::new(owner, installed, source) {
        Ok(host) => host,
        Err(error) => {
            tracing::warn!(%error, "could not bind agent panel context");
            return Ok(());
        }
    };
    let enabled = host
        .settings()
        .setting::<nocterm_ai::AiSettings>(cx)
        .enabled;
    let panel = cx.new(|panel_cx| panel::AgentPanel::new(host, window, panel_cx));
    workspace.register_panel(
        nocterm_workspace::PanelRegistration::with_entity(&plugin::CHATS, panel).expandable(),
        window,
        cx,
    )?;
    workspace.set_panel_available(&plugin::CHATS.id, enabled, window, cx);
    Ok(())
}
