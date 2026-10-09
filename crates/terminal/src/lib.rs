//! The terminal tab: a session's screen, keyboard and prompts.
//!
//! Two layers, so that the screen can be read without a window:
//!
//! - [`Terminal`] is the model. It owns the [`Session`](nocterm_session::Session)
//!   and the emulator, pumps events from one into the other, carries out what
//!   the emulator asks of the host (replies, clipboard, bell) and holds the
//!   question the session is waiting on. A future collaboration or AI feature
//!   observes this entity, not the view.
//! - [`TerminalView`] is a workspace [`Item`](nocterm_workspace::Item): it
//!   paints the grid, turns keys, mouse and IME input into bytes, and shows
//!   prompts and the session's status.
//!
//! The crate never names a transport. The application installs one with
//! [`init`], and the workspace reaches this crate through [`open_session`],
//! installed as its session opener, and [`open_program`], its program opener.

mod access;
mod codec;
mod credentials;
mod recording;
pub use recording::RecordingStatus;
struct RecordingDirectory {
    path: std::path::PathBuf,
    registry: std::sync::Arc<recording::Registry>,
}
impl gpui_kit::Global for RecordingDirectory {}
/// Sets the default output-log directory at the composition root.
pub fn init_recording(directory: std::path::PathBuf, cx: &mut gpui_kit::App) {
    let registry = std::sync::Arc::new(recording::Registry::default());
    cx.set_global(RecordingDirectory {
        path: directory,
        registry: registry.clone(),
    });
    cx.on_app_quit(move |cx| {
        registry.stop_all();
        let registry = registry.clone();
        let executor = cx.background_executor().clone();
        async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
            while !registry.finished() && std::time::Instant::now() < deadline {
                executor.timer(std::time::Duration::from_millis(10)).await;
            }
            if !registry.finished() {
                tracing::warn!("timed out flushing session output logs");
            }
        }
    })
    .detach();
}
mod element;
mod highlighting;
pub use credentials::init_credentials;
mod integration;
mod session_settings;
mod terminal;
mod view;

use std::sync::Arc;

use gpui_kit::{
    Action as _, App, Context, Global, KeyBinding, NoAction, Unbind, Window, prelude::*,
};
use nocterm_session::{ShellLaunch, Transport};
use nocterm_workspace::{ProgramSpec, SessionSpec, Workspace};

pub use terminal::{FindState, Status, Terminal, TerminalEvent};
pub use view::TerminalView;

gpui_kit::actions!(
    terminal,
    [
        /// Copy the selected text.
        Copy,
        /// Paste the clipboard into the terminal.
        Paste,
        /// Scroll back by one screen.
        ScrollPageUp,
        /// Scroll forward by one screen.
        ScrollPageDown,
        /// Scroll to the oldest line of history.
        ScrollToTop,
        /// Scroll back to the live screen.
        ScrollToBottom,
        /// Reconnect this tab using its next-launch options.
        Reconnect,
        /// Start or stop output-only session recording.
        ToggleRecording,
    ]
);

/// The key context of a terminal, for keymap entries.
pub const KEY_CONTEXT: &str = "Terminal";

/// The screen itself; authentication inputs keep normal form navigation.
pub(crate) const SCREEN_KEY_CONTEXT: &str = "TerminalScreen";

struct ActiveTransport(Arc<dyn Transport>);

impl Global for ActiveTransport {}

type LocalFactory = Arc<dyn Fn(ShellLaunch) -> Arc<dyn Transport> + Send + Sync>;
struct LocalTransportFactory(LocalFactory);
impl Global for LocalTransportFactory {}

/// Installs a platform local-session adapter without coupling this feature to it.
pub fn init_local(factory: LocalFactory, cx: &mut App) {
    cx.set_global(LocalTransportFactory(factory));
}

/// Opens a shell in the requested live workspace pane.
pub fn open_local(
    workspace: &mut Workspace,
    target: nocterm_workspace::LocalTerminalTarget,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let view = cx.new(|cx| TerminalView::new_local(window, cx));
    workspace.add_local_terminal_at(view, target, window, cx);
}

/// Opens `spec` without a tab.
pub fn open_background_session(
    workspace: &mut Workspace,
    spec: SessionSpec,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> gpui_kit::EntityId {
    let view = cx.new(|cx| TerminalView::new(spec, window, cx));
    workspace.add_background_item(view, window, cx)
}

/// Opens a dedicated command tab without replacing the workspace's bottom shell.
pub fn open_local_command(
    workspace: &mut Workspace,
    launch: ShellLaunch,
    title: String,
    transport: Arc<dyn Transport>,
    completion: impl FnOnce(nocterm_session::CloseReason, &mut App) + 'static,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let view = cx.new(|cx| {
        TerminalView::new_local_command(launch, title, transport, completion, window, cx)
    });
    workspace.add_item(view, window, cx);
}

/// Opens `spec`'s program in a new tab.
pub fn open_program(
    workspace: &mut Workspace,
    spec: ProgramSpec,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let view = cx.new(|cx| {
        let terminal = cx.new(|cx| Terminal::new_program(spec, cx));
        TerminalView::with_terminal(terminal, window, cx)
    });
    workspace.add_item(view, window, cx);
}

/// Installs the transport terminals open their sessions with.
pub fn init(transport: Arc<dyn Transport>, _: &nocterm_ui::UiReady, cx: &mut App) {
    nocterm_ui::register_setting::<nocterm_session::LoggingOptions>(cx);
    nocterm_ui::register_setting::<nocterm_session::SshSettings>(cx);
    nocterm_ui::register_setting::<nocterm_session::LocalShellSettings>(cx);
    // Keep inherited focus traversal and native Copy shortcuts off the screen.
    // Target only native Copy so explicit terminal actions retain precedence.
    cx.bind_keys([
        KeyBinding::new("tab", NoAction, Some(SCREEN_KEY_CONTEXT)),
        KeyBinding::new("shift-tab", NoAction, Some(SCREEN_KEY_CONTEXT)),
        KeyBinding::new(
            "ctrl-c",
            Unbind(gpui_kit::component::input::Copy.name().into()),
            Some(SCREEN_KEY_CONTEXT),
        ),
    ]);
    cx.set_global(ActiveTransport(transport));
}

/// Opens every kind of tab as a terminal; the workspace's session factory.
pub struct Factory;

impl nocterm_workspace::SessionFactory for Factory {
    fn open_session(
        &self,
        workspace: &mut Workspace,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        open_session(workspace, spec, window, cx);
    }
    fn open_background(
        &self,
        workspace: &mut Workspace,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Option<gpui_kit::EntityId> {
        Some(open_background_session(workspace, spec, window, cx))
    }
    fn open_local(
        &self,
        workspace: &mut Workspace,
        target: nocterm_workspace::LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        open_local(workspace, target, window, cx);
    }
    fn open_program(
        &self,
        workspace: &mut Workspace,
        spec: ProgramSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        open_program(workspace, spec, window, cx);
    }
}

/// Opens `spec` in a new terminal tab.
pub fn open_session(
    workspace: &mut Workspace,
    spec: SessionSpec,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let view = cx.new(|cx| TerminalView::new(spec, window, cx));
    workspace.add_item(view, window, cx);
}
