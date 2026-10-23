//! Native catalog row actions retain the exact original definition.
use crate::{Installed, context, editor};
use gpui_kit::{
    App, SharedString, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_project_model::ProjectDefinition;
use nocterm_ui::IconName;
use nocterm_workspace::Workspace;

const ROW_GROUP: &str = "project-row";

#[derive(IntoElement)]
pub(super) struct ProjectRow {
    actions: RowActions,
}
impl ProjectRow {
    pub(super) fn new(
        definition: ProjectDefinition,
        installed: Installed,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        Self {
            actions: RowActions {
                definition,
                installed,
                workspace,
            },
        }
    }
    fn caption(
        &self,
        server: Option<nocterm_workspace_ports::ProjectParentDescriptor>,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let definition = &self.actions.definition;
        h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(Icon::new(IconName::FolderTree).small().flex_shrink_0())
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "project-row-name-{}",
                                definition.id()
                            )))
                            .test_support()
                            .aria_label(definition.name().to_owned())
                            .w_full()
                            .text_sm()
                            .truncate()
                            .child(definition.name().to_owned()),
                    )
                    .when_some(server, |caption, server| {
                        caption.child(self.secondary(server, cx))
                    }),
            )
    }
    fn secondary(
        &self,
        server: nocterm_workspace_ports::ProjectParentDescriptor,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let definition = &self.actions.definition;
        h_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .id(SharedString::from(format!(
                        "project-row-server-{}",
                        definition.id()
                    )))
                    .test_support()
                    .aria_label(server.title().clone())
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(server.title().clone()),
            )
            .when_some(
                nocterm_project_providers::ContainerLocation::from_target(definition.target()).ok(),
                |line, location| {
                    let engine = match location.engine() {
                        nocterm_containers::Engine::Docker => "Docker",
                        nocterm_containers::Engine::Podman => "Podman",
                    };
                    line.child(
                        div()
                            .id(SharedString::from(format!(
                                "project-row-engine-{}",
                                definition.id()
                            )))
                            .test_support()
                            .aria_label(engine)
                            .flex_shrink_0()
                            .px_1()
                            .rounded_sm()
                            .bg(cx.theme().muted)
                            .text_xs()
                            .child(engine),
                    )
                },
            )
    }

    fn actions(&self) -> impl IntoElement + use<> {
        let edit = self.actions.clone();
        let remove = self.actions.clone();
        h_flex()
            .gap_1()
            .flex_shrink_0()
            .invisible()
            .group_hover(ROW_GROUP, |actions| actions.visible())
            .child(
                Button::new(SharedString::from(format!(
                    "project-edit-{}",
                    edit.definition.id()
                )))
                .ghost()
                .xsmall()
                .icon(IconName::Pencil)
                .tooltip("Workspace settings")
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    editor::edit(
                        edit.installed.clone(),
                        edit.definition.clone(),
                        edit.workspace.clone(),
                        window,
                        cx,
                    );
                }),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "project-remove-{}",
                    remove.definition.id()
                )))
                .ghost()
                .xsmall()
                .icon(IconName::Trash)
                .tooltip("Remove saved workspace")
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    remove.confirm_remove(window, cx);
                }),
            )
    }
}
impl RenderOnce for ProjectRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let open = self.actions.clone();
        let server = self.actions.workspace.upgrade().and_then(|workspace| {
            let workspace = workspace.read(cx);
            let current = context::current(workspace, &open.installed, window, cx);
            context::server(
                workspace,
                &crate::location::host(open.definition.target())?,
                current.as_ref(),
                cx,
            )
            .ok()
        });
        h_flex()
            .id(SharedString::from(format!(
                "project-row-{}",
                open.definition.id()
            )))
            .test_support()
            .group(ROW_GROUP)
            .mx_1()
            .px_2()
            .py_2()
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "project-open-{}",
                        open.definition.id()
                    )))
                    .test_support()
                    .aria_label(open.definition.name().to_owned())
                    .py_1()
                    .flex_1()
                    .min_w_0()
                    .child(self.caption(server, cx)),
            )
            .child(self.actions())
            .on_click(move |_, window, cx| open.open(window, cx))
    }
}

#[derive(Clone)]
struct RowActions {
    definition: ProjectDefinition,
    installed: Installed,
    workspace: WeakEntity<Workspace>,
}
impl RowActions {
    fn open(&self, window: &mut Window, cx: &mut App) {
        if let Some(workspace) = self.workspace.upgrade() {
            let result = workspace.update(cx, |workspace, cx| {
                self.installed
                    .open(self.definition.id(), workspace, window, cx)
            });
            if let Err(error) = result {
                nocterm_ui::notice::error(
                    window,
                    cx,
                    "project-open",
                    "Could not open workspace",
                    error.to_string(),
                );
            }
        }
    }
    fn confirm_remove(&self, window: &mut Window, cx: &mut App) {
        crate::removal::open(
            self.installed.clone(),
            self.definition.clone(),
            self.workspace.clone(),
            window,
            cx,
        );
    }
}
