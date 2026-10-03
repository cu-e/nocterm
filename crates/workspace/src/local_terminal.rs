//! Local shell bridge. Explorer depends on this contract, never Terminal.
use crate::Item;
use gpui_kit::{App, Context, Entity, Window};
use std::path::PathBuf;

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
