//! Shared explicitly supplied session Items for action/event and multi-window tests.
//! Owns: focus and supplied session/generation transitions. Does not own: transport,
//! native work, settings/global installation or a terminal implementation.
use crate::{
    ExpectedSession, Item, ItemEvent, SessionContext, SessionSpec, TerminalCapabilities,
    TerminalInfo, TerminalState, TerminalStatus, Workspace,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, WeakEntity,
    Window, div, prelude::*,
};
use nocterm_session::RemoteFs;
use std::{rc::Rc, sync::Arc};
/// Extracted existing workspace Remote fixture; all mutable state stays private.
pub struct SessionProbe {
    focus: FocusHandle,
    spec: SessionSpec,
    connected: bool,
    fs: Option<Arc<dyn RemoteFs>>,
    exec: Option<Arc<dyn nocterm_session::HostExec>>,
    generation: u64,
    parent: Option<Rc<dyn nocterm_workspace_ports::ProjectParentCapture>>,
}
#[derive(Debug, thiserror::Error)]
#[error("test session generation is exhausted")]
pub struct SessionProbeError;
impl SessionProbe {
    pub fn new(spec: SessionSpec, connected: bool, cx: &mut Context<Workspace>) -> Entity<Self> {
        Self::with_fs(spec, connected, None, cx)
    }
    pub fn with_fs(
        spec: SessionSpec,
        connected: bool,
        fs: Option<Arc<dyn RemoteFs>>,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        Self::from_app(spec, connected, fs, cx)
    }
    /// Compose the same Item from a narrow opening port, without a Workspace borrow.
    pub fn from_app(
        spec: SessionSpec,
        connected: bool,
        fs: Option<Arc<dyn RemoteFs>>,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self {
            focus: cx.focus_handle(),
            spec,
            connected,
            fs,
            exec: None,
            generation: 0,
            parent: None,
        })
    }
    /// Supply structured execution without a native session in feature tests.
    pub fn set_executor(
        &mut self,
        exec: Arc<dyn nocterm_session::HostExec>,
        cx: &mut Context<Self>,
    ) {
        self.exec = Some(exec);
        cx.emit(ItemEvent::Changed);
    }
    pub fn set_project_parent(
        &mut self,
        parent: Rc<dyn nocterm_workspace_ports::ProjectParentCapture>,
        cx: &mut Context<Self>,
    ) {
        self.parent = Some(parent);
        cx.emit(ItemEvent::Changed);
    }
    pub fn set_connected(
        &mut self,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), SessionProbeError> {
        if connected != self.connected {
            self.generation = self.generation.checked_add(1).ok_or(SessionProbeError)?;
            self.connected = connected;
            cx.emit(ItemEvent::Changed);
            cx.notify();
        }
        Ok(())
    }
    /// Remember/association may change a profile without reconnecting transport.
    pub fn associate_profile(
        &mut self,
        profile: Option<gpui_kit::SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.spec.profile = profile;
        cx.emit(ItemEvent::Changed);
        cx.notify();
    }
    pub fn replace_session(
        &mut self,
        spec: SessionSpec,
        fs: Option<Arc<dyn RemoteFs>>,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), SessionProbeError> {
        let generation = self.generation.checked_add(1).ok_or(SessionProbeError)?;
        self.spec = spec;
        self.fs = fs;
        self.connected = connected;
        self.generation = generation;
        cx.emit(ItemEvent::Changed);
        cx.notify();
        Ok(())
    }
}
impl EventEmitter<ItemEvent> for SessionProbe {}
impl Focusable for SessionProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for SessionProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus)
    }
}
impl Item for SessionProbe {
    fn on_close(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.set_connected(false, cx)
            .expect("test session generation is available");
    }
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        self.spec.title.clone()
    }
    fn session(&self, _: &App) -> Option<SessionContext> {
        Some(
            SessionContext::new(self.spec.target.clone(), self.fs.clone(), self.connected)
                .with_via(self.spec.address().via)
                .with_exec(self.exec.clone()),
        )
    }
    fn session_spec(&self, _: &App) -> Option<SessionSpec> {
        Some(self.spec.clone())
    }
    fn terminal_capabilities(&self, owner: WeakEntity<Self>) -> Option<TerminalCapabilities> {
        let state = Rc::new(ProbeState { owner });
        Some(
            TerminalCapabilities::new(state.clone())
                .with_fact_session(state.clone())
                .with_project_parent(state.clone())
                .with_execution(state),
        )
    }
}
impl crate::LocalTerminal for SessionProbe {
    fn cwd(&self, _: &App) -> Option<std::path::PathBuf> {
        self.spec
            .launch
            .as_ref()?
            .cwd
            .as_ref()
            .map(std::path::PathBuf::from)
    }
    fn change_directory(
        &mut self,
        _: std::path::PathBuf,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Result<(), crate::LocalDirectoryError> {
        Err(crate::LocalDirectoryError::UnsupportedShell)
    }
}
struct ProbeState {
    owner: WeakEntity<SessionProbe>,
}
impl nocterm_workspace_ports::ProjectParentCapture for ProbeState {
    fn capture(
        &self,
        expected: ExpectedSession,
        cx: &App,
    ) -> Result<
        nocterm_workspace_ports::ProjectCreationSource,
        nocterm_workspace_ports::ProjectParentError,
    > {
        use nocterm_workspace_ports::ProjectParentError;
        let owner = self
            .owner
            .upgrade()
            .ok_or(ProjectParentError::Disconnected)?;
        let probe = owner.read(cx);
        if expected != ExpectedSession::new(owner.entity_id(), probe.generation) {
            return Err(ProjectParentError::StaleSession);
        }
        if !probe.connected {
            return Err(ProjectParentError::Disconnected);
        }
        probe
            .parent
            .as_ref()
            .ok_or(ProjectParentError::Disconnected)?
            .capture(expected, cx)
    }
}
impl crate::TerminalExecution for ProbeState {
    fn executor(
        &self,
        expected: ExpectedSession,
        cx: &App,
    ) -> Result<Arc<dyn nocterm_session::HostExec>, crate::TerminalError> {
        let owner = self.owner.upgrade().ok_or(crate::TerminalError::Gone)?;
        let probe = owner.read(cx);
        if expected != ExpectedSession::new(owner.entity_id(), probe.generation) {
            return Err(crate::TerminalError::StaleSession);
        }
        if !probe.connected {
            return Err(crate::TerminalError::Unavailable);
        }
        probe.exec.clone().ok_or(crate::TerminalError::Unsupported(
            crate::TerminalCapability::SessionExecution,
        ))
    }
}
impl crate::ConnectionFactSession for ProbeState {
    fn current(&self, cx: &App) -> Option<crate::FactSessionSnapshot> {
        let owner = self.owner.upgrade()?;
        let probe = owner.read(cx);
        Some(crate::FactSessionSnapshot {
            expected: ExpectedSession::new(owner.entity_id(), probe.generation),
            session: SessionContext::new(
                probe.spec.target.clone(),
                probe.fs.clone(),
                probe.connected,
            )
            .with_via(probe.spec.address().via),
            profile: probe.spec.profile.clone(),
        })
    }
}
impl TerminalState for ProbeState {
    fn info(&self, cx: &App) -> Option<TerminalInfo> {
        let owner = self.owner.upgrade()?;
        let probe = owner.read(cx);
        Some(TerminalInfo {
            title: probe.spec.title.clone(),
            local: false,
            target: Some(probe.spec.target.clone()),
            profile: probe.spec.profile.clone(),
            status: if probe.connected {
                TerminalStatus::Connected
            } else {
                TerminalStatus::Closed
            },
            cwd: None,
            at_prompt: None,
            dirty_input: false,
            alt_screen: false,
            generation: probe.generation,
            expected: ExpectedSession::new(owner.entity_id(), probe.generation),
            sign_in: None,
        })
    }
}

mod project_terminal;
pub use project_terminal::ProjectTerminalProbe;

mod project_surface;
pub use project_surface::ProjectSurfaceProbe;

mod panel;
pub use panel::{TestPanel, TestPanelBuilder};

mod plugin;
pub use plugin::{
    CORE_PANEL, EXPANDABLE, PANEL_A, PANEL_B, REQUIRED_EXPANDABLE, REQUIRED_PLUGIN, RIGHT_WIDGET,
    SIDEBAR_EXPANDABLE, TEST_CORE, TEST_PLUGIN, TRAY_PANEL, TRAY_WIDGET, WIDGET, deps_for,
    deps_for_core,
};

mod shell;
pub use shell::{Toggle, set_side_panel_attention, toggle_id, toggle_presentation};
