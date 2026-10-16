//! Actual bootstrap windows retain the same supplied snippet installation.
use super::*;
mod native;
fn show_library(
    workspace: &Entity<Workspace>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    workspace.update(cx, |workspace, cx| {
        workspace.activate_panel_of::<nocterm_snippets_ui::SnippetsPanel>(window, cx)
    });
    window.render_frame(cx);
}
fn create_shared_snippet(
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_snippets_ui::NewSnippet), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("Shared original", cx);
        window.click("snippet-code", cx);
        window.input("echo shared", cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
}
fn isolated_snippets(cx: &mut gpui_kit::App) -> nocterm_snippets_ui::Installed {
    let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
    let model = nocterm_snippets_store::SnippetsHandle::install(
        nocterm_snippets_store::PreparedLibrary::memory(),
        std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
        &nocterm_store::ShutdownCoordinator::default(),
        cx,
    )
    .unwrap();
    nocterm_snippets_ui::init(
        model,
        settings,
        std::sync::Arc::new(nocterm_snippets::test_support::SequenceIds::new(50_000)),
        cx,
    )
}

#[gpui_kit::test]
fn snippet_installation_survives_first_window_and_remains_shared_with_new_windows(
    cx: &mut TestAppContext,
) {
    let (booted, first, first_workspace, second, second_workspace) = cx.update(|cx| {
        let booted = boot(
            tempfile::tempdir().unwrap(),
            Default::default(),
            Arc::new(MockTransport::default()),
            false,
            cx,
        );
        let (first, first_workspace) = open_workspace(&booted, cx);
        let (second, second_workspace) = open_workspace(&booted, cx);
        (booted, first, first_workspace, second, second_workspace)
    });
    let first_weak = first_workspace.downgrade();
    create_shared_snippet(first, &first_workspace, cx);
    cx.run_until_parked();
    cx.update_window(second, |_, window, cx| {
        window.activate_window();
        show_library(&second_workspace, window, cx);
        window.click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "echo shared"
        );
    })
    .unwrap();
    let separate = cx.update(isolated_snippets);
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    drop(first_workspace);
    cx.run_until_parked();
    assert!(
        first_weak.upgrade().is_none(),
        "closed workspace must release its subscriptions"
    );
    let (third, third_workspace) = cx.update(|cx| open_workspace(&booted, cx));
    cx.update_window(third, |_, window, cx| {
        window.activate_window();
        show_library(&third_workspace, window, cx);
        window.click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "echo shared"
        );
        window.click("edit", cx);
        window.render_frame(cx);
        window.click("snippet-code", cx);
        window.press("ctrl-a", cx);
        window.input("echo from third", cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(second, |_, window, cx| {
        window.activate_window();
        show_library(&second_workspace, window, cx);
        window.click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "echo from third"
        );
        assert!(separate.library(cx).snippets.is_empty());
    })
    .unwrap();
}
