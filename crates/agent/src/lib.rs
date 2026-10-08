//! ACP chat UI and lifecycle, injected through domain contracts.
mod panel;
mod runtime;
mod thread;
pub use runtime::{AgentServices, TerminalAuthOpener, TerminalAuthRequest};

use gpui_kit::{App, prelude::*};

/// Installs services without starting an agent or opening a listener.
pub fn init(services: AgentServices, cx: &mut App) {
    panel::commands::init(cx);
    let runtime = cx.new(|cx| runtime::Runtime::new(services, cx));
    cx.set_global(runtime::RuntimeGlobal(runtime.clone()));
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
