//! A bounded rendered catalog snapshot; grouping rules belong to project-model.
mod row;
use crate::Installed;
use gpui_kit::{
    AnyElement, App, ClickEvent, WeakEntity, Window,
    component::{
        ActiveTheme as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_project_model::{ProjectDefinition, ProjectGroups};
use nocterm_ui::IconName;
use nocterm_workspace::Workspace;
use row::ProjectRow;

pub(super) struct ProjectLibrary {
    current: Vec<ProjectDefinition>,
    other: Vec<ProjectDefinition>,
    folded: bool,
    searching: bool,
}
impl ProjectLibrary {
    pub(super) fn new(groups: ProjectGroups<'_>, folded: bool, searching: bool) -> Self {
        Self {
            current: groups.current.into_iter().cloned().collect(),
            other: groups.other.into_iter().cloned().collect(),
            folded,
            searching,
        }
    }
    pub(super) fn render(
        self,
        installed: Installed,
        workspace: WeakEntity<Workspace>,
        on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let empty = self.current.is_empty() && self.other.is_empty();
        let count = self.other.len();
        let row = |definition| ProjectRow::new(definition, installed.clone(), workspace.clone());
        v_flex()
            .id("projects-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(!self.current.is_empty(), |list| {
                list.child(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .font_medium()
                        .text_color(theme.muted_foreground)
                        .child("For this host"),
                )
                .children(self.current.into_iter().map(row))
            })
            .when(empty, |list| {
                list.child(
                    div()
                        .px_3()
                        .py_4()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(if self.searching {
                            "No matching workspaces."
                        } else {
                            "Use + to create a workspace on this host."
                        }),
                )
            })
            .when(count > 0, |list| {
                list.child(
                    Button::new("projects-other")
                        .ghost()
                        .small()
                        .w_full()
                        .justify_start()
                        .icon(if self.folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .label(format!("All other workspaces ({count})"))
                        .on_click(on_toggle),
                )
                .when(!self.folded, |list| {
                    list.children(self.other.into_iter().map(row))
                })
            })
            .into_any_element()
    }
}
