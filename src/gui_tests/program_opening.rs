use super::*;
use nocterm_session::testing::{FakeExec, InMemoryFs};

#[gpui_kit::test]
fn explorer_in_new_windows_uses_the_opener_supplied_once_to_bootstrap(cx: &mut TestAppContext) {
    let native = Arc::new(FakeExec::default());
    let local = Arc::new(InMemoryFs::default());
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("note.txt");
    seed_native_ancestors(&local, directory.path());
    local.seed_native_file(&file, b"file".to_vec()).unwrap();
    let mut settings = nocterm_settings::SettingsDocument::default();
    settings.update::<nocterm_session::LocalShellSettings>(|section| {
        section.cwd = Some(directory.path().to_string_lossy().into_owned());
    });
    settings.update::<nocterm_files::ExplorerSettings>(|section| {
        section.open.local = nocterm_files::Opener::new("injected-editor", &[]);
    });
    let deps = nocterm_files::Deps::new(
        local.clone(),
        local,
        Arc::new(nocterm_transfers::testing::FakeTransfers::default()),
        native.clone(),
    );
    let first = cx.update(|cx| {
        let booted = boot_with_files(
            directory,
            settings,
            Arc::new(MockTransport::default()),
            false,
            Some(deps),
            cx,
        );
        open_workspace(&booted, cx).0
    });
    open_local_entry(cx, first);
    cx.update_window(first, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::NewWindow), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 2);
    let second = windows.into_iter().find(|window| *window != first).unwrap();
    open_local_entry(cx, second);
    let requests = native.opened_requests();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.program(), "injected-editor");
        assert_eq!(request.args(), [file.to_string_lossy()]);
        assert_eq!(request.cwd(), file.parent());
    }
}

fn open_local_entry(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.simulate_window_resize(
        handle,
        gpui_kit::size(gpui_kit::px(1400.), gpui_kit::px(1100.)),
    );
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        let workspace = gpui_kit::base::Root::read(window, cx)
            .view()
            .clone()
            .downcast::<Workspace>()
            .unwrap();
        window.focus(&workspace.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_files::ToggleExplorer), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let position = window.find(("local-entry", 0usize)).bounds().center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                position,
                button: gpui_kit::MouseButton::Left,
                modifiers: Default::default(),
                click_count: 2,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}
