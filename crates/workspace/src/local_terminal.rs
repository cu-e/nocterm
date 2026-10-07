//! Local shell bridge. Explorer depends on this contract, never Terminal.
use crate::Item;
use gpui_kit::{App, Context, Entity, EntityId, Window};
use std::path::PathBuf;

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
    ) -> Result<(), String>;
}
pub(crate) trait LocalTerminalHandle {
    fn cwd(&self, cx: &App) -> Option<PathBuf>;
    fn change_directory(
        &self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String>;
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
    ) -> Result<(), String> {
        self.update(cx, |this, cx| this.change_directory(path, window, cx))
    }
}
