//! ACP chat UI and lifecycle, injected through domain contracts.
mod panel;
use nocterm_agent_runtime as runtime;
mod thread;
pub use runtime::TerminalAuthRequest;

use gpui_kit::{App, Global, WeakEntity, Window, prelude::*};
use nocterm_ai::{AgentConnector, ToolBridge};
use nocterm_ui::SettingsExt as _;
use std::{path::PathBuf, sync::Arc};

/// Runs an agent's sign-in command in a terminal of `workspace`, and reports
/// whether it succeeded.
pub type TerminalAuthOpener = Arc<
    dyn Fn(
            WeakEntity<nocterm_workspace::Workspace>,
            TerminalAuthRequest,
            &mut Window,
            &mut App,
        ) -> futures::channel::oneshot::Receiver<Result<(), String>>
        + Send
        + Sync,
>;

/// What the host provides to the agents.
pub struct AgentServices {
    pub connector: Arc<dyn AgentConnector>,
    pub bridge: Arc<dyn ToolBridge>,
    pub state_file: PathBuf,
    /// Saved chats, one file each.
    pub chats_dir: PathBuf,
    /// Where Codex keeps its session logs, read for its plan limits.
    pub codex_home: Option<PathBuf>,
    pub workdir: PathBuf,
    pub terminal_auth: Option<TerminalAuthOpener>,
    /// nocterm's own directories (settings, vault, chats), hidden from
    /// isolated agents.
    pub private_dirs: Vec<PathBuf>,
    /// Directories isolated agents must still reach, such as the terminal
    /// tools' socket directory.
    pub shared_dirs: Vec<PathBuf>,
}

/// The host's sign-in terminal, if it has one.
#[derive(Clone, Default)]
pub(crate) struct TerminalAuth(pub Option<TerminalAuthOpener>);

impl Global for TerminalAuth {}

impl TerminalAuth {
    pub(crate) fn opener(cx: &App) -> Option<TerminalAuthOpener> {
        cx.try_global::<Self>().and_then(|auth| auth.0.clone())
    }
}

/// Installs services without starting an agent or opening a listener.
pub fn init(services: AgentServices, cx: &mut App) {
    panel::commands::init(cx);
    cx.set_global(TerminalAuth(services.terminal_auth.clone()));
    cx.set_global(runtime::AiSettingsSource(|cx| {
        cx.setting::<nocterm_ai::AiSettings>()
    }));
    let services = runtime::RuntimeServices {
        connector: services.connector,
        bridge: services.bridge,
        state_file: services.state_file,
        chats_dir: services.chats_dir,
        codex_home: services.codex_home,
        workdir: services.workdir,
        terminal_auth: services.terminal_auth.is_some(),
        private_dirs: services.private_dirs,
        shared_dirs: services.shared_dirs,
    };
    let runtime = cx.new(|cx| runtime::Runtime::new(services, cx));
    cx.set_global(runtime::RuntimeGlobal(runtime.clone()));
    let observed = runtime.downgrade();
    cx.observe_global::<nocterm_ui::SettingsStore>(move |cx| {
        let _ = observed.update(cx, |runtime, cx| runtime.settings_changed(cx));
    })
    .detach();
    cx.on_app_quit(move |cx| runtime.update(cx, |runtime, cx| runtime.shutdown(cx)))
        .detach();
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
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<nocterm_workspace::Workspace>,
) {
    let owner = cx.entity().downgrade();
    let panel = cx.new(|panel_cx| panel::AgentPanel::new(owner, window, panel_cx));
    workspace.set_right_panel(panel, window, cx);
    workspace.set_right_panel_available(nocterm_ui::ActiveAi::ai_enabled(&**cx), window, cx);
}
