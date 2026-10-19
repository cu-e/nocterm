//! Local shell bridge. Explorer depends on this contract, never Terminal.
use crate::Item;
use gpui_kit::{App, Context, Entity, EntityId, Window};
use std::path::PathBuf;

/// Refusal to synchronize a workspace's local shell directory.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalDirectoryError {
    #[error("Local terminal is unavailable")]
    Unavailable,
    #[error("Open a connected local terminal first.")]
    NotConnectedLocal,
    #[error("The shell cannot represent this directory path.")]
    NonUtf8Path,
    #[error("Directory paths with control characters require manual navigation.")]
    ControlCharacters,
    #[error(
        "Directory synchronization needs Bash, Zsh, fish or PowerShell with shell integration enabled."
    )]
    UnsupportedShell,
    #[error("The shell is busy or has unfinished input. Run at an empty prompt: {command}")]
    Busy { command: String },
    #[error("{0}")]
    Encoding(#[from] nocterm_session::TextEncodingError),
    #[error("Directory change was not sent because the terminal input queue is full.")]
    QueueFull,
}

/// Destination for a local shell request; anchors are resolved when it registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalTerminalTarget {
    /// The selected bottom pane, or a new bottom region when none exists.
    Bottom,
    /// The live pane containing this workspace Item, wherever it has moved.
    Beside(EntityId),
}

pub trait LocalTerminal: Item {
    fn cwd(&self, cx: &App) -> Option<PathBuf>;
    /// Return an error when foreground work or unsupported shell integration
    /// makes sending a directory change unsafe.
    fn change_directory(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), LocalDirectoryError>;
}
pub(crate) trait LocalTerminalHandle {
    fn cwd(&self, cx: &App) -> Option<PathBuf>;
    fn change_directory(
        &self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), LocalDirectoryError>;
}
impl<T: LocalTerminal> LocalTerminalHandle for Entity<T> {
    fn cwd(&self, cx: &App) -> Option<PathBuf> {
        self.read(cx).cwd(cx)
    }
    fn change_directory(
        &self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), LocalDirectoryError> {
        self.update(cx, |this, cx| this.change_directory(path, window, cx))
    }
}
