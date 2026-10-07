//! Selecting the terminal last focused by the user, including the bottom shell.
use super::Workspace;
use gpui_kit::{App, Context, EntityId, Window, component::dock::DockPlacement};
use std::path::PathBuf;

impl Workspace {
    pub fn local_terminal_cwd(&self, cx: &App) -> Option<PathBuf> {
        self.selected_local(cx)
            .and_then(|open| open.local.as_ref()?.cwd(cx))
    }

    pub fn local_terminal_is_visible(&self, cx: &App) -> bool {
        self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
            && self.items.iter().any(|open| {
                self.item_location(open.handle.item_id(), cx)
                    .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            })
    }

    /// Focus exactly this visible user terminal, selecting its central tab if
    /// necessary. A missing, background or hidden target never falls back.
    pub fn focus_terminal(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.items
            .iter()
            .any(|open| open.handle.item_id() == id && open.handle.terminal_access(cx).is_some())
            && self.activate_item_by_id(id, window, cx)
    }

    pub fn active_terminal(&self, cx: &App) -> Option<EntityId> {
        let terminals = self.terminals(cx);
        self.last_command_item
            .filter(|id| terminals.iter().any(|entry| entry.item == *id))
            .or_else(|| {
                self.active_item()
                    .filter(|item| item.terminal_access(cx).is_some())
                    .map(|item| item.item_id())
            })
    }

    pub fn terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        let mut entries: Vec<_> = self
            .items
            .iter()
            .filter_map(|open| {
                Some(crate::TerminalEntry {
                    item: open.handle.item_id(),
                    access: open.handle.terminal_access(cx)?,
                    title: open
                        .dock_item
                        .read(cx)
                        .alias
                        .clone()
                        .unwrap_or_else(|| open.handle.tab_title(cx)),
                    active: false,
                    bottom: self
                        .item_location(open.handle.item_id(), cx)
                        .is_some_and(|(placement, _)| placement == DockPlacement::Bottom),
                    background: false,
                })
            })
            .collect();
        entries.extend(self.background_entries(cx));
        let active = self
            .last_command_item
            .filter(|id| entries.iter().any(|entry| entry.item == *id))
            .or_else(|| self.active_item().map(|item| item.item_id()));
        for entry in &mut entries {
            entry.active = active == Some(entry.item);
        }
        entries
    }
}
