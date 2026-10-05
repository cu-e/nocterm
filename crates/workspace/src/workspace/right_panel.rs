//! Hosting, visibility and attention of the independent side panel.

use gpui_kit::{Context, Entity, Window};

use super::{Workspace, WorkspaceEvent};

impl Workspace {
    pub fn set_right_panel<T: crate::RightPanel>(
        &mut self,
        panel: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_panel_subscription = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, event, window, cx| match event {
                crate::RightPanelEvent::ToggleMaximized => {
                    this.set_right_panel_maximized(!this.right_panel_maximized, cx)
                }
                crate::RightPanelEvent::Close => this.close_right_panel(window, cx),
                crate::RightPanelEvent::Widen(delta) => this.widen_right_panel(*delta, window, cx),
            },
        ));
        panel.update(cx, |panel, cx| {
            panel.set_docked_left(Self::sides_swapped(cx), cx)
        });
        self.right_panel = Some(Box::new(panel));
        cx.notify();
    }

    pub fn set_right_panel_available(
        &mut self,
        available: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_panel_available == available {
            return;
        }
        self.right_panel_available = available;
        if !available {
            self.close_right_panel(window, cx);
        }
        cx.notify();
    }

    pub fn set_right_panel_attention(&mut self, attention: bool, cx: &mut Context<Self>) {
        if self.right_panel_attention != attention {
            self.right_panel_attention = attention;
            cx.notify();
        }
    }
    pub fn right_panel_is_available(&self) -> bool {
        self.right_panel_available && self.right_panel.is_some()
    }
    pub fn right_panel_is_open(&self) -> bool {
        self.right_panel_open
    }
    pub fn right_panel_is_maximized(&self) -> bool {
        self.right_panel_maximized
    }

    pub fn toggle_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.right_panel_is_available() {
            return;
        }
        if self.right_panel_open {
            self.close_right_panel(window, cx);
        } else {
            self.right_panel_open = true;
            cx.emit(WorkspaceEvent::RightPanelVisibilityChanged);
            if let Some(panel) = &self.right_panel {
                window.focus(&panel.focus_handle(cx), cx);
            }
            cx.notify();
        }
    }

    pub fn set_right_panel_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        let maximized = maximized && self.right_panel_is_available() && self.right_panel_open;
        if self.right_panel_maximized == maximized {
            return;
        }
        self.right_panel_maximized = maximized;
        if let Some(panel) = &self.right_panel {
            panel.set_maximized(maximized, cx);
        }
        cx.notify();
    }

    pub(super) fn close_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self
            .right_panel
            .as_ref()
            .is_some_and(|panel| panel.focus_handle(cx).contains_focused(window, cx));
        self.set_right_panel_maximized(false, cx);
        let was_open = self.right_panel_open;
        self.right_panel_open = false;
        if was_open {
            cx.emit(WorkspaceEvent::RightPanelVisibilityChanged);
        }
        if had_focus {
            let focus = self
                .active_item()
                .map(|item| item.focus_handle(cx))
                .unwrap_or_else(|| self.focus_handle.clone());
            window.focus(&focus, cx);
        }
        cx.notify();
    }
}
