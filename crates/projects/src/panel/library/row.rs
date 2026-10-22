//! Native catalog row actions retain the exact original definition.
use crate::{Installed, context, editor};
use gpui_kit::{
    App, SharedString, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_project_model::ProjectDefinition;
use nocterm_ui::IconName;
use nocterm_workspace::Workspace;

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
                            .truncate()
                            .child(definition.name().to_owned()),
                    )
                    .when_some(server, |caption, server| {
                        caption.child(
                            div()
                                .id(SharedString::from(format!(
                                    "project-row-server-{}",
                                    definition.id()
                                )))
                                .test_support()
                                .aria_label(server.title().clone())
                                .w_full()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .truncate()
                                .child(server.title().clone()),
                        )
                    }),
            )
    }
    fn actions(&self) -> impl IntoElement + use<> {
        let edit = self.actions.clone();
        let remove = self.actions.clone();
        h_flex()
            .gap_1()
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
                    editor::edit(edit.installed.clone(), edit.definition.clone(), window, cx);
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
                .on_click(move |_, window, cx| remove.confirm_remove(window, cx)),
            )
    }
}
impl RenderOnce for ProjectRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let open = self.actions.clone();
        let server = self.actions.workspace.upgrade().and_then(|workspace| {
            let workspace = workspace.read(cx);
            let current = context::current(workspace, window, cx);
            context::server(workspace, open.definition.host(), current.as_ref(), cx).ok()
        });
        h_flex()
            .id(SharedString::from(format!(
                "project-row-{}",
                open.definition.id()
            )))
            .test_support()
            .px_3()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border.opacity(0.5))
            .child(
                Button::new(SharedString::from(format!(
                    "project-open-{}",
                    open.definition.id()
                )))
                .ghost()
                .small()
                .h_auto()
                .py_1()
                .px_0()
                .flex_1()
                .min_w_0()
                .accessibility_label(open.definition.name().to_owned())
                .child(self.caption(server, cx))
                .tooltip(open.definition.root().as_str().to_owned())
                .on_click(move |_, window, cx| open.open(window, cx)),
            )
            .child(self.actions())
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
        let actions = self.clone();
        window.open_alert_dialog(cx,move |dialog,_,_| {
            let actions=actions.clone();
            dialog.title(format!("Remove workspace “{}”?",actions.definition.name()))
                .description("Removes saved workspace settings. Open terminals, project files and chat history are kept.")
                .ok_text("Remove workspace").show_cancel(true).on_ok(move |_,window,cx| {
                    let removed=actions.installed.model().remove(actions.definition.clone(),cx);
                    window.spawn(cx,async move |cx| {if let Err(error)=removed.await {
                        let _=cx.update(|window,cx|nocterm_ui::notice::error(window,cx,"project-remove","Could not remove workspace",error.to_string()));
                    }}).detach();
                    true
                })
        });
    }
}
