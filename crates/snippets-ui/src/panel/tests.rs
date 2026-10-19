use super::*;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_ui::{Design, DesignTokens};

fn fixture(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Entity<SnippetsPanel>,
    Entity<Workspace>,
    Installed,
) {
    let (handle, workspace, installed) = cx.update(|cx| {
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let installed = crate::test_support::install(
            settings,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps =
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
                    Workspace::new(deps, window, cx)
                })
            })
            .unwrap();
        (handle, workspace, installed)
    });
    let panel = cx
        .update_window(handle, |_, window, cx| {
            window.activate_window();
            let panel =
                cx.new(|cx| SnippetsPanel::new(installed.clone(), workspace.clone(), window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_panel(panel.clone(), cx);
                workspace.activate_panel_of::<SnippetsPanel>(window, cx);
            });
            panel
        })
        .unwrap();
    cx.run_until_parked();
    (handle, panel, workspace, installed)
}

fn snippet() -> Snippet {
    Snippet {
        name: "Disk usage 日本語".into(),
        description: "Описание команды".into(),
        content: "#!/bin/bash\necho 'привет'\n\n".into(),
        groups: vec!["Missing group".into()],
        profiles: vec!["missing-server".into()],
        ..Snippet::new(uuid::Uuid::from_u128(1))
    }
}

fn seed(snippet: &Snippet, installed: &Installed, cx: &mut TestAppContext) {
    cx.update(|cx| {
        installed.save(snippet.clone(), None, cx).detach();
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn plus_opens_editor_and_cancel_preserves_library(cx: &mut TestAppContext) {
    let (handle, _, _, installed) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-new", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        window.click("snippet-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert!(installed.library(cx).snippets.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn fold_search_copy_and_edit_preserve_exact_saved_snippet(cx: &mut TestAppContext) {
    let (handle, _, _, installed) = fixture(cx);
    let original = snippet();
    seed(&original, &installed, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("copy").is_some());
        window.click("snippets-other", cx);
        window.render_frame(cx);
        assert!(window.try_find("copy").is_none());
        window.click("snippets-other", cx);
        window.render_frame(cx);
        window.click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            original.content
        );
        window.click("snippet-library-search", cx);
        window.input("unmatched query", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("copy").is_none());
        window.press("ctrl-a", cx);
        window.input("ПРИВЕТ", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("copy").is_some());
        window.click("edit", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        window.click("snippet-section-attachments", cx);
        window.render_frame(cx);
        assert!(window.try_find("profile-missing-server").is_some());
        assert!(window.try_find("group-Missing group").is_some());
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert_eq!(
            installed.library(cx).snippets.as_slice(),
            std::slice::from_ref(&original)
        );
        window.click("edit", cx);
        window.render_frame(cx);
        window.click("snippet-section-attachments", cx);
        window.render_frame(cx);
        window.click("profile-missing-server", cx);
        window.click("group-Missing group", cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let mut unassigned = original;
        unassigned.profiles.clear();
        unassigned.groups.clear();
        assert_eq!(installed.library(cx).snippets, [unassigned]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn deletion_requires_separate_confirmation_and_cancel_keeps_snippet(cx: &mut TestAppContext) {
    let (handle, _, _, installed) = fixture(cx);
    let original = snippet();
    seed(&original, &installed, cx);
    let separate = cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        crate::test_support::install(settings, &nocterm_store::ShutdownCoordinator::default(), cx)
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("delete", cx);
        window.render_frame(cx);
        assert!(window.try_find("snippet-save").is_none());
        assert!(window.try_find("ok").is_some());
        assert_eq!(
            installed.library(cx).snippets.as_slice(),
            std::slice::from_ref(&original)
        );
        window.click("cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(installed.library(cx).snippets, [original]);
        window.click("delete", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ok").is_none());
        assert!(installed.library(cx).snippets.is_empty());
        assert!(separate.library(cx).snippets.is_empty());
    })
    .unwrap();
}
#[path = "run_tests.rs"]
mod run_tests;
