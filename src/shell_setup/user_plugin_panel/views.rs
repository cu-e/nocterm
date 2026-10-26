//! The panel and the widget of a user plugin: the plugin's view in its frame, refreshed while
//! it shows.
use gpui_kit::{
    App, Context, FocusHandle, Focusable, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Task, WeakEntity, Window,
    component::{tooltip::Tooltip, v_flex},
    div,
    prelude::*,
};
use nocterm_plugin_model::ContributionId;
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace};

use super::content::{Content, drive};

/// The panel of a user plugin contribution.
pub(super) struct UserPluginPanel {
    content: Content,
    id: ContributionId,
    title: SharedString,
    icon: IconName,
    badge: Option<u16>,
    focus: FocusHandle,
    focused: bool,
    workspace: WeakEntity<Workspace>,
    _refresh: Task<()>,
}

impl UserPluginPanel {
    /// The first look comes from the refresh loop, after the update that creates the panel
    /// ends: the loop reads the workspace (`tick`), which is being updated right now. Only a
    /// task run between updates may read it; the workspace provider of the host does not.
    pub(super) fn new(
        content: Content,
        id: ContributionId,
        title: SharedString,
        icon: IconName,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let refresh = drive(cx, |this: &mut Self, cx| this.tick(cx));
        Self {
            content,
            id,
            title,
            icon,
            badge: None,
            focus: cx.focus_handle(),
            focused: false,
            workspace,
            _refresh: refresh,
        }
    }

    pub(super) fn set_badge(&mut self, badge: Option<u16>, cx: &mut Context<Self>) {
        if self.badge != badge {
            self.badge = badge;
            cx.notify();
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let shown = self
            .workspace
            .read_with(cx, |workspace, _| workspace.panel_is_open(&self.id))
            .unwrap_or(false);
        self.content.poll(shown, self.focused, cx);
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
}

impl Focusable for UserPluginPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for UserPluginPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focused = self.focus.contains_focused(window, cx);
        v_flex()
            .id("user-plugin-panel")
            .size_full()
            .track_focus(&self.focus)
            .child(self.content.view().clone())
    }
}

/// The widget of a user plugin contribution in the status bar.
pub(super) struct UserPluginWidget {
    content: Content,
    label: SharedString,
    /// Whether the bar drew the widget since the last refresh. A widget the bar did not draw
    /// (overflowed while the menu is closed, or the bar hidden) asks the plugin for nothing
    /// until it is drawn again, which the open overflow menu does with the widget's own view.
    drawn: bool,
    _refresh: Task<()>,
}

impl UserPluginWidget {
    /// `label` is what the tooltip says: the plugin, then the widget. The first look is asked
    /// for at once, before the bar has drawn the widget.
    pub(super) fn new(content: Content, label: SharedString, cx: &mut Context<Self>) -> Self {
        let refresh = drive(cx, |this: &mut Self, cx| this.tick(cx));
        Self {
            content,
            label,
            drawn: true,
            _refresh: refresh,
        }
    }

    /// A refresh clears the mark and asks the bar to draw the widget again, which sets it: an
    /// idle window draws nothing, so only a refresh can tell that the widget still shows.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.content.poll(self.drawn, false, cx) {
            self.drawn = false;
            cx.notify();
        }
    }
}

impl Render for UserPluginWidget {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.drawn = true;
        let label = self.label.clone();
        div()
            .id("user-plugin-widget")
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .child(self.content.view().clone())
    }
}
