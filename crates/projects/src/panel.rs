//! The original current parent precedes the collapsible rest of the library.
mod library;
use crate::{Installed, context, editor};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use library::ProjectLibrary;
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace, WorkspaceEvent};
use nocterm_workspace_ports::ProjectParentDescriptor;

pub struct ProjectsPanel {
    installed: Installed,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    search: Entity<InputState>,
    parent: Option<ProjectParentDescriptor>,
    can_create: bool,
    folded: bool,
    _subscriptions: Vec<Subscription>,
}
impl ProjectsPanel {
    pub(crate) fn new(
        installed: Installed,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search workspaces…"));
        let weak = cx.weak_entity();
        let native_window = window.window_handle();
        let mut subscriptions = vec![
            installed.observe(cx, move |cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            }),
            cx.observe(&search, |_, _, cx| cx.notify()),
        ];
        if let Some(owner) = workspace.upgrade() {
            subscriptions.push(cx.subscribe(&owner, move |_, _, event, cx| {
                if matches!(
                    event,
                    WorkspaceEvent::ActiveItemChanged
                        | WorkspaceEvent::ActiveSessionChanged
                        | WorkspaceEvent::ConnectionsChanged
                        | WorkspaceEvent::ItemsChanged
                        | WorkspaceEvent::ContextChanged
                ) {
                    let weak = cx.weak_entity();
                    cx.defer(move |cx| {
                        let _ = native_window.update(cx, |_, window, cx| {
                            let _ = weak.update(cx, |this, cx| this.refresh(window, cx));
                        });
                    });
                }
            }));
        }
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = native_window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.refresh(window, cx));
            });
        });
        Self {
            installed,
            workspace,
            focus: cx.focus_handle(),
            search,
            parent: None,
            can_create: false,
            folded: false,
            _subscriptions: subscriptions,
        }
    }
    fn refresh(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.parent = self.workspace.upgrade().and_then(|workspace| {
            context::current(workspace.read(cx), &self.installed, window, cx)
        });
        self.can_create = self.workspace.upgrade().is_some_and(|workspace| {
            context::capture_current(workspace.read(cx), &self.installed, window, cx).is_ok()
        });
        cx.notify();
    }
    fn create(&self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self
            .workspace
            .upgrade()
            .ok_or(nocterm_workspace_ports::ProjectParentError::Disconnected)
            .and_then(|workspace| {
                context::capture_current(workspace.read(cx), &self.installed, window, cx)
            });
        match source {
            Ok(source) => {
                editor::open(self.installed.clone(), source, window, cx);
            }
            Err(error) => nocterm_ui::notice::error(
                window,
                cx,
                "project-create",
                "Select a connected server",
                error.to_string(),
            ),
        }
    }
    fn toggle_other(&mut self, cx: &mut Context<Self>) {
        self.folded = !self.folded;
        cx.notify();
    }
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let title = self.parent.as_ref().map_or_else(
            || SharedString::from("Select a connected server"),
            |parent| parent.title().clone(),
        );
        h_flex()
            .px_3()
            .gap_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .id("project-context")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .aria_label(title.clone())
                    .child(title),
            )
            .child(
                Button::new("project-new")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("New workspace on this host")
                    .disabled(!self.installed.model().writable(cx) || !self.can_create)
                    .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
            )
    }
}
impl Panel for ProjectsPanel {
    fn title(&self, _: &App) -> SharedString {
        "Workspaces".into()
    }
    fn icon(&self, _: &App) -> IconName {
        IconName::Layers
    }
}
impl Focusable for ProjectsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ProjectsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let parent_key = self
            .parent
            .as_ref()
            .and_then(|parent| parent.host().parent_key().ok());
        let query = self.search.read(cx).value();
        let groups =
            self.installed
                .catalog(cx)
                .partition_by(parent_key.as_ref(), &query, |target| {
                    crate::location::host(target).and_then(|host| host.parent_key().ok())
                });
        let library = ProjectLibrary::new(groups, self.folded, !query.trim().is_empty());
        let theme = cx.theme().clone();
        let error = self
            .installed
            .model()
            .error(cx)
            .map(|error| error.to_string());
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .gap_1()
            .child(self.header(cx))
            .child(
                div()
                    .id("project-search")
                    .test_support()
                    .px_3()
                    .py_1()
                    .child(Input::new(&self.search).small()),
            )
            .when_some(error, |panel, error| {
                panel.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .child(library.render(
                self.installed.clone(),
                self.workspace.clone(),
                cx.listener(|this, _, _, cx| this.toggle_other(cx)),
                cx,
            ))
    }
}
