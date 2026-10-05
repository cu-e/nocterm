//! The sidebar's panels and the strip that switches between them.

use gpui_kit::{Context, Entity, Window};

use super::Workspace;
use crate::Panel;

impl Workspace {
    /// Adds a sidebar panel, after the ones already there.
    pub fn add_panel<T: Panel>(&mut self, panel: Entity<T>, cx: &mut Context<Self>) {
        // A panel's title, icon and badge sit in the workspace's chrome; the
        // rest of the panel redraws itself, so only a change to them counts.
        let chrome = |panel: &Entity<T>, cx: &gpui_kit::App| {
            let panel = panel.read(cx);
            (panel.title(cx), panel.icon(cx), panel.badge(cx))
        };
        let mut shown = chrome(&panel, cx);
        cx.observe(&panel, move |_, panel, cx| {
            let now = chrome(&panel, cx);
            if now != shown {
                shown = now;
                cx.notify();
            }
        })
        .detach();
        self.panels.push(Box::new(panel));
        cx.notify();
    }

    /// Shows the panel at `ix`, opening the sidebar if it is closed.
    pub fn activate_panel(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        let Some(panel) = self.panels.get(ix) else {
            return;
        };
        self.active_panel = ix;
        self.sidebar_open = true;
        window.focus(&panel.focus_handle(cx), cx);
        cx.notify();
    }

    /// Shows the panel showing a `T`, if there is one.
    pub fn activate_panel_of<T: Panel>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.panel_index_of::<T>() {
            self.activate_panel(ix, window, cx);
        }
    }

    /// Shows the panel showing a `T`, or hides the sidebar when it already
    /// shows it.
    pub fn toggle_panel_of<T: Panel>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.panel_index_of::<T>() {
            Some(ix) if self.sidebar_open && self.active_panel == ix => {
                self.toggle_sidebar(window, cx)
            }
            Some(ix) => self.activate_panel(ix, window, cx),
            None => {}
        }
    }

    fn panel_index_of<T: Panel>(&self) -> Option<usize> {
        self.panels
            .iter()
            .position(|panel| panel.view().downcast::<T>().is_ok())
    }

    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        self.sidebar_open = !self.sidebar_open;
        if !self.sidebar_open
            && let Some(item) = self.active_item()
        {
            let focus = item.focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
