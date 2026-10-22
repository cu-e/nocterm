use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, TestAppContext, WindowOptions,
    test::TestWindowExt as _,
};
use nocterm_project_model::*;
use nocterm_project_service::{PreparedProjects, ProjectsHandle};
use nocterm_ui::{Design, DesignTokens};
use nocterm_workspace::{ItemHandle as _, ProjectTerminalOpen, Workspace};

use crate::*;

mod creation;
mod library;
mod lifecycle;
mod support;
mod terminals;

use support::{definition, fixture};

#[gpui_kit::test]
fn creation_without_an_open_connection_keeps_the_saved_library(cx: &mut TestAppContext) {
    let (window, _, installed) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-name").is_none());
        assert_eq!(installed.catalog(cx).iter().count(), 2);
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn two_saved_workspaces_open_as_distinct_outer_items_and_repeated_open_focuses_existing(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, installed) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
            installed
                .open(definition(2).id(), workspace, window, cx)
                .unwrap();
            installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
            assert_eq!(workspace.items(cx).count(), 2);
            assert_eq!(
                workspace.selected_context(cx),
                ContextId::Project(definition(1).id())
            );
            assert_eq!(
                workspace.active_item(cx).unwrap().tab_title(cx).as_ref(),
                "Workspace 1"
            );
        });
        window.render_frame(cx);
        assert!(window.try_find("project-unavailable").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_and_fold_hide_rows_without_removing_saved_workspaces(cx: &mut TestAppContext) {
    let (handle, _, installed) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("projects-other", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_none()
        );
        window.click("projects-other", cx);
        window.click("project-search", cx);
        window.input("Workspace 2", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_none()
        );
        assert!(
            window
                .try_find("project-open-02020202020202020202020202020202")
                .is_some()
        );
        assert_eq!(installed.catalog(cx).iter().count(), 2);
    })
    .unwrap();
}
