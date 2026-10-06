//! Selecting the terminal last focused by the user, including the bottom shell.
use super::Workspace;
use gpui_kit::{App, Context, EntityId, Window, component::dock::DockPlacement};
use std::path::PathBuf;

impl Workspace {
    pub fn local_terminal_cwd(&self, cx: &App) -> Option<PathBuf> {
        self.local_terminal
            .as_ref()
            .and_then(|local| local.local.cwd(cx))
    }

    pub fn local_terminal_is_visible(&self, cx: &App) -> bool {
        self.local_terminal
            .as_ref()
            .is_some_and(|local| local.attached)
            && self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
    }

    /// Focus exactly this visible user terminal, selecting its central tab if
    /// necessary. A missing, background or hidden target never falls back.
    pub fn focus_terminal(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(index) = self.items.iter().position(|item| {
            item.handle.item_id() == id && item.handle.terminal_access(cx).is_some()
        }) {
            self.last_command_item = Some(id);
            self.activate_item(index, window, cx);
            return true;
        }
        if self.local_terminal_is_visible(cx)
            && let Some(local) = self.local_terminal.as_ref().filter(|local| {
                local.handle.item_id() == id && local.handle.terminal_access(cx).is_some()
            })
        {
            let focus = local.handle.focus_handle(cx);
            self.last_command_item = Some(id);
            window.focus(&focus, cx);
            cx.notify();
            return true;
        }
        false
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
                    bottom: false,
                    background: false,
                })
            })
            .collect();
        if let Some(local) = &self.local_terminal
            && let Some(access) = local.handle.terminal_access(cx)
        {
            entries.push(crate::TerminalEntry {
                item: local.handle.item_id(),
                access,
                title: local.handle.tab_title(cx),
                active: false,
                bottom: true,
                background: false,
            });
        }
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
