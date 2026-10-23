//! The terminal tab: a session's screen, keyboard and prompts.
//! Shell integration and live search facts stay private and expose immutable observations.
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
//! [`install`], and the workspace reaches this crate through [`open_session`],
//! installed as its session opener, and [`open_program`], its program opener.
//! Text conversion remains here; directory synchronization preserves session
//! charset errors through the workspace LocalTerminal contract.

//! Input refusals preserve typed charset and queue errors through workspace contracts.
//! Weak Item and model handles validate original connection and prompt tokens before
//! input; replacement connections never inherit approval or sign-in authority.

//! Owns: private session lifecycle, admitted input and live leases, screen/search
//! transitions, credential candidates, output recording and per-view paint/input owners.
//! Does not own: native transport, filesystem, vault or persistence execution.
//! Ports: session transport and credential contracts; narrow workspace terminal capabilities.
//! Original parent capture and ProjectTerminalOpen are narrow folder-creation
//! ports. Workspace sessions bind their checked target before opening surfaces.
//! ContainerSessionFactory arrives explicitly with installed dependencies and
//! adapts the original transport; container sessions expose no host filesystem
//! or parent-creation capture. Prompts and reconnect keep the ordinary lifecycle.
//! WorkspaceSessionFactory separately activates managed tools for folder workspace
//! shells and execution, retaining the host filesystem and original launch settings.
//! Ordinary server terminals keep their supplied transports.
//! Settings: TerminalSettings plus SSH, local launch and recording defaults.
//! Globals: none. Installed capabilities and the UiReady settings handle are explicit.
//! Invariants: captured connection/prompt authority precedes effects; input is admitted
//! exactly once before integration/lease changes; paint snapshots track presentation revisions;
//! a jump host's prompt is answered only from that hop's own saved credential and never remembered.
//! Every public General opener captures its original surface/window before native
//! creation. Folder launches validate their original inner surface destination.
//! Ordinary local opening returns the exact registered ItemId for human workspace
//! recovery; the unchanged General factory maps it to its existing unit result.

//! Settings pages: terminal, local-shell and SSH controls retain the original Installed settings.
mod access;
mod codec;
mod credentials;
mod file_drop;
mod recording;
pub(crate) use recording::OutputRecording;
pub use recording::RecordingStatus;

mod element;
mod highlighting;
pub use credentials::CredentialAssociationError;
mod installed;
pub use installed::{CredentialProvider, Deps, Installed, LocalCommand, LocalSessions};
mod integration;
mod session_settings;
mod terminal;
mod view;

use gpui_kit::{Action as _, App, Context, KeyBinding, NoAction, Unbind, Window, prelude::*};
use nocterm_workspace::ContextSelectionError;
use nocterm_workspace::{ProgramSpec, SessionSpec, Workspace};
mod project;
pub use project::{local_project_session, project_session, project_terminal_opener};

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

/// Opens a General shell in the requested original outer pane or local dock.
pub fn open_local(
    installed: &Installed,
    workspace: &mut Workspace,
    target: nocterm_workspace::LocalTerminalTarget,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<gpui_kit::EntityId, ContextSelectionError> {
    let origin = workspace.general_terminal_target(window, cx)?;
    if let nocterm_workspace::LocalTerminalTarget::Beside(anchor) = target
        && workspace.tab_location(anchor, cx).is_none()
    {
        return Err(ContextSelectionError::StaleSurface);
    }
    let view = cx.new(|cx| TerminalView::new_local(installed.clone(), window, cx));
    let item = view.entity_id();
    workspace.add_general_local_terminal_from(origin.selection(), view, target, window, cx)?;
    Ok(item)
}

/// Opens `spec` without a tab.
pub fn open_background_session(
    installed: &Installed,
    workspace: &mut Workspace,
    spec: SessionSpec,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<gpui_kit::EntityId, ContextSelectionError> {
    let origin = workspace.general_terminal_target(window, cx)?;
    let view = cx.new(|cx| TerminalView::new(installed.clone(), spec, window, cx));
    workspace.add_general_background_from(origin.selection(), view, window, cx)
}

/// Opens a dedicated command tab without replacing the workspace's bottom shell.
pub fn open_local_command(
    installed: &Installed,
    workspace: &mut Workspace,
    command: LocalCommand,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), ContextSelectionError> {
    let origin = workspace.general_terminal_target(window, cx)?;
    let (launch, title, transport, completion) = command.into_parts();
    let view = cx.new(|cx| {
        TerminalView::new_local_command(
            installed.clone(),
            launch,
            title,
            transport,
            completion,
            window,
            cx,
        )
    });
    workspace.add_general_item_from(origin.selection(), view, window, cx)?;
    Ok(())
}

/// Opens `spec`'s program in a new tab.
pub fn open_program(
    installed: &Installed,
    workspace: &mut Workspace,
    spec: ProgramSpec,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), ContextSelectionError> {
    let origin = workspace.general_terminal_target(window, cx)?;
    let view = cx.new(|cx| {
        let terminal = cx.new(|cx| Terminal::new_program(installed.clone(), spec, cx));
        TerminalView::with_terminal(terminal, window, cx)
    });
    workspace.add_general_item_from(origin.selection(), view, window, cx)?;
    Ok(())
}

/// Installs the transport terminals open their sessions with.
pub fn install(deps: Deps, ready: &nocterm_ui::UiReady, cx: &mut App) -> Installed {
    ready
        .settings()
        .register::<nocterm_terminal_model::TerminalSettings>(cx);
    ready
        .settings()
        .register::<nocterm_session::LoggingOptions>(cx);
    ready
        .settings()
        .register::<nocterm_session::SshSettings>(cx);
    ready
        .settings()
        .register::<nocterm_session::LocalShellSettings>(cx);
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
    deps.install(ready.settings().clone())
}

/// Bind the same installed capabilities to each workspace window.
pub fn register(installed: &Installed, workspace: &mut Workspace) {
    workspace.set_session_factory(Factory {
        installed: installed.clone(),
    });
}

/// Opens every kind of tab as a terminal; the workspace's session factory.
struct Factory {
    installed: Installed,
}

impl nocterm_workspace::SessionFactory for Factory {
    fn open_session(
        &self,
        workspace: &mut Workspace,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        report(open_session(&self.installed, workspace, spec, window, cx));
    }
    fn open_background(
        &self,
        workspace: &mut Workspace,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Option<gpui_kit::EntityId> {
        match open_background_session(&self.installed, workspace, spec, window, cx) {
            Ok(id) => Some(id),
            Err(error) => {
                report(Err(error));
                None
            }
        }
    }
    fn open_local(
        &self,
        workspace: &mut Workspace,
        target: nocterm_workspace::LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        report(open_local(&self.installed, workspace, target, window, cx).map(|_| ()));
    }
    fn open_program(
        &self,
        workspace: &mut Workspace,
        spec: ProgramSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        report(open_program(&self.installed, workspace, spec, window, cx));
    }
}

/// Opens `spec` in a new terminal tab.
pub fn open_session(
    installed: &Installed,
    workspace: &mut Workspace,
    spec: SessionSpec,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), ContextSelectionError> {
    let origin = workspace.general_terminal_target(window, cx)?;
    let view = cx.new(|cx| TerminalView::new(installed.clone(), spec, window, cx));
    workspace.add_general_item_from(origin.selection(), view, window, cx)?;
    Ok(())
}
fn report(result: Result<(), ContextSelectionError>) {
    if let Err(error) = result {
        tracing::warn!(?error, "terminal opening refused");
    }
}

mod settings_page;
pub use settings_page::settings_pages;

#[cfg(test)]
mod opening_tests;
