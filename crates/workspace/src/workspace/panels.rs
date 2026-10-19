//! The sidebar's panels and the strip that switches between them.

use gpui_kit::{AnyView, App, EntityId, FocusHandle, SharedString, Subscription};

use crate::{Panel, PanelHandle};
use nocterm_ui::IconName;

// A sidebar owns both its cached presentation metadata and each observer's lifetime.
pub(super) struct SidebarEntry {
    handle: Box<dyn PanelHandle>,
    metadata: (SharedString, IconName, Option<SharedString>),
    _subscription: Subscription,
}

impl SidebarEntry {
    pub(super) fn title(&self, _: &App) -> SharedString {
        self.metadata.0.clone()
    }
    pub(super) fn icon(&self, _: &App) -> IconName {
        self.metadata.1
    }
    pub(super) fn badge(&self, _: &App) -> Option<SharedString> {
        self.metadata.2.clone()
    }
    pub(super) fn view(&self) -> AnyView {
        self.handle.view()
    }
}

pub(super) struct Sidebar {
    entries: Vec<SidebarEntry>,
    selected: usize,
    visible: bool,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            visible: true,
        }
    }
}

impl Sidebar {
    pub(super) fn register(
        &mut self,
        handle: Box<dyn PanelHandle>,
        metadata: (SharedString, IconName, Option<SharedString>),
        subscription: Subscription,
    ) {
        self.entries.push(SidebarEntry {
            handle,
            metadata,
            _subscription: subscription,
        });
    }
    pub(super) fn metadata_changed(
        &mut self,
        id: EntityId,
        metadata: (SharedString, IconName, Option<SharedString>),
    ) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.handle.panel_id() == id)
        else {
            return false;
        };
        if entry.metadata == metadata {
            return false;
        }
        entry.metadata = metadata;
        true
    }
    pub(super) fn activate(&mut self, ix: usize, cx: &App) -> Option<FocusHandle> {
        let focus = self.entries.get(ix)?.handle.focus_handle(cx);
        self.selected = ix;
        self.visible = true;
        Some(focus)
    }
    pub(super) fn toggle(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }
    pub(super) fn visible(&self) -> bool {
        self.visible
    }
    pub(super) fn showing(&self, ix: usize) -> bool {
        self.visible && self.selected == ix
    }
    pub(super) fn active(&self) -> Option<&SidebarEntry> {
        self.entries.get(self.selected)
    }
    pub(super) fn entries(&self) -> impl Iterator<Item = &SidebarEntry> {
        self.entries.iter()
    }
    pub(super) fn index_of<T: Panel>(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.handle.view().downcast::<T>().is_ok())
    }
    pub(super) fn next(&self) -> Option<usize> {
        if self.entries.is_empty() {
            None
        } else if self.visible {
            Some((self.selected + 1) % self.entries.len())
        } else {
            Some(self.selected)
        }
    }
}

#[cfg(test)]
mod tests;
