//! The panel and the widget of a user plugin: the plugin's view in its frame, refreshed while
//! it shows.
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, WeakEntity, Window,
    component::{tooltip::Tooltip, v_flex},
    div,
    prelude::*,
};
use nocterm_plugin_model::ContributionId;
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Provenance, Workspace};

use super::content::Content;

/// The panel of a user plugin contribution.
pub(super) struct UserPluginPanel {
    content: Entity<Content>,
    title: SharedString,
    icon: IconName,
    badge: Option<u16>,
    focus: FocusHandle,
}

impl UserPluginPanel {
    /// Visibility is reconciled after the workspace update ends. The shared driver can then
    /// read panel membership without borrowing the workspace while it creates this view.
    pub(super) fn new(
        content: Entity<Content>,
        id: ContributionId,
        title: SharedString,
        icon: IconName,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        content.update(cx, |content, _| content.panel(workspace, id.clone()));
        Self {
            content,
            title,
            icon,
            badge: None,
            focus: cx.focus_handle(),
        }
    }

    pub(super) fn set_badge(&mut self, badge: Option<u16>, cx: &mut Context<Self>) {
        if self.badge != badge {
            self.badge = badge;
            cx.notify();
        }
    }
}

impl Panel for UserPluginPanel {
    fn title(&self, _: &App) -> SharedString {
        self.title.clone()
    }

    fn icon(&self, _: &App) -> IconName {
        self.icon
    }

    fn badge(&self, _: &App) -> Option<SharedString> {
        self.badge.map(|count| count.to_string().into())
    }

    /// "Plugin" beside the title, "Plugin · DEV" in a developer session; the tooltip names the
    /// plugin.
    fn provenance(&self, cx: &App) -> Option<Provenance> {
        let view = self.content.read(cx).view().read(cx);
        let chip = if view.is_dev() {
            "Plugin \u{b7} DEV"
        } else {
            "Plugin"
        };
        Some(Provenance {
            chip: chip.into(),
            detail: view.label(),
        })
    }
}

impl Focusable for UserPluginPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for UserPluginPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.contains_focused(window, cx);
        self.content
            .update(cx, |content, cx| content.visible(true, focused, cx));
        v_flex()
            .id("user-plugin-panel")
            .size_full()
            .track_focus(&self.focus)
            .child(self.content.read(cx).view().clone())
    }
}

/// The widget of a user plugin contribution in the status bar.
pub(super) struct UserPluginWidget {
    content: Entity<Content>,
    label: SharedString,
}

impl UserPluginWidget {
    /// `label` identifies the plugin and widget in its tooltip. Actual bar/overflow
    /// visibility starts and stops its interest in the shared window driver.
    pub(super) fn new(content: Entity<Content>, label: SharedString) -> Self {
        Self { content, label }
    }

    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.content
            .update(cx, |content, cx| content.visible(visible, false, cx));
    }
}

impl Render for UserPluginWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.content.update(cx, |content, cx| content.restyle(cx));
        let label = self.label.clone();
        div()
            .id("user-plugin-widget")
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .child(self.content.read(cx).view().clone())
    }
}
