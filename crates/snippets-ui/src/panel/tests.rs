use super::*;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_ui::{Design, DesignTokens};

fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<SnippetsPanel>) {
    let (handle, workspace) = cx.update(|cx| {
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        crate::init(None, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        })
        .unwrap()
    });
    let panel = cx
        .update_window(handle, |_, window, cx| {
            window.activate_window();
            let panel = cx
                .new(|cx| SnippetsPanel::new(Snippets::global(cx), workspace.clone(), window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_panel(panel.clone(), cx);
                workspace.activate_panel_of::<SnippetsPanel>(window, cx);
            });
            panel
        })
        .unwrap();
    cx.run_until_parked();
    (handle, panel)
}

fn snippet() -> Snippet {
    Snippet {
        name: "Disk usage 日本語".into(),
        description: "Описание команды".into(),
        content: "#!/bin/bash\necho 'привет'\n\n".into(),
        groups: vec!["Missing group".into()],
        profiles: vec!["missing-server".into()],
        ..Default::default()
    }
}

fn seed(snippet: &Snippet, cx: &mut TestAppContext) {
    cx.update(|cx| {
        Snippets::global(cx).update(cx, |model, cx| {
            model.library.save(snippet.clone(), None).unwrap();
            cx.notify();
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn plus_opens_editor_and_cancel_preserves_library(cx: &mut TestAppContext) {
    let (handle, _) = fixture(cx);
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
        assert!(Snippets::global(cx).read(cx).library.snippets.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn fold_search_copy_and_edit_preserve_exact_saved_snippet(cx: &mut TestAppContext) {
    let (handle, panel) = fixture(cx);
    let original = snippet();
    seed(&original, cx);
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
        let focus = panel.read(cx).search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
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
            Snippets::global(cx).read(cx).library.snippets.as_slice(),
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
        assert_eq!(Snippets::global(cx).read(cx).library.snippets, [unassigned]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn deletion_requires_separate_confirmation_and_cancel_keeps_snippet(cx: &mut TestAppContext) {
    let (handle, _) = fixture(cx);
    let original = snippet();
    seed(&original, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("delete", cx);
        window.render_frame(cx);
        assert!(window.try_find("snippet-save").is_none());
        assert!(window.try_find("ok").is_some());
        assert_eq!(
            Snippets::global(cx).read(cx).library.snippets.as_slice(),
            std::slice::from_ref(&original)
        );
        window.click("cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(Snippets::global(cx).read(cx).library.snippets, [original]);
        window.click("delete", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ok").is_none());
        assert!(Snippets::global(cx).read(cx).library.snippets.is_empty());
    })
    .unwrap();
}

#[path = "run_tests.rs"]
mod run_tests;
