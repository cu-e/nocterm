use super::*;
use futures::executor::block_on;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_explorer::testing::MemoryDirectoryReader;
use nocterm_session::{FsError, RemoteFs, testing::InMemoryFs};

fn fixture(
    cx: &mut TestAppContext,
    source: Arc<InMemoryFs>,
) -> (AnyWindowHandle, Entity<DirectoryPicker>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init(
            crate::DesignTokens::builtin(),
            nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                DirectoryPicker::new(
                    Arc::new(MemoryDirectoryReader::remote(source)),
                    None,
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    })
}
#[gpui_kit::test]
fn chosen_directory_is_valid_only_after_a_successful_actual_listing(cx: &mut TestAppContext) {
    let source = Arc::new(InMemoryFs::default());
    block_on(source.create_dir("/資料 folder")).unwrap();
    let (handle, picker) = fixture(cx, source);
    cx.run_until_parked();
    assert_eq!(
        picker.read_with(cx, |picker, cx| picker.value(cx)),
        Some(Directory::Remote("/".into()))
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-path-display", cx);
        window.input("/資料 folder", cx);
    })
    .unwrap();
    assert!(!picker.read_with(cx, |picker, cx| picker.is_valid(cx)));
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        picker.read_with(cx, |picker, cx| picker.value(cx)),
        Some(Directory::Remote("/資料 folder".into()))
    );
}
#[gpui_kit::test]
fn failed_directory_preserves_the_literal_draft_and_has_no_valid_value(cx: &mut TestAppContext) {
    let (handle, picker) = fixture(cx, Arc::new(InMemoryFs::default()));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-path-display", cx);
        window.input("/missing 資料", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        picker.read_with(cx, |picker, cx| picker.draft(cx)),
        "/missing 資料"
    );
    assert!(
        picker
            .read_with(cx, |picker, cx| picker.value(cx))
            .is_none()
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("explorer-path-error").visible());
    })
    .unwrap();
}
#[gpui_kit::test]
fn replacing_the_source_discards_a_pending_old_server_listing(cx: &mut TestAppContext) {
    let old = Arc::new(InMemoryFs::default());
    old.script_listings();
    let (_, picker) = fixture(cx, old.clone());
    cx.run_until_parked();
    let pending = old.take_pending_listing().unwrap();
    let current = Arc::new(InMemoryFs::default());
    block_on(current.create_dir("/current")).unwrap();
    current.set_home("/current").unwrap();
    picker.update(cx, |picker, cx| {
        picker.replace_reader(Arc::new(MemoryDirectoryReader::remote(current)), None, cx)
    });
    cx.run_until_parked();
    let _ = pending.send(Err(FsError::NotConnected));
    cx.run_until_parked();
    assert_eq!(
        picker.read_with(cx, |picker, cx| picker.value(cx)),
        Some(Directory::Remote("/current".into()))
    );
}
#[gpui_kit::test]
fn home_up_and_refresh_share_the_same_directory_actions(cx: &mut TestAppContext) {
    let source = Arc::new(InMemoryFs::default());
    block_on(source.create_dir("/home")).unwrap();
    source.set_home("/home").unwrap();
    let (handle, picker) = fixture(cx, source);
    cx.run_until_parked();
    for (control, expected) in [
        ("directory-parent", "/"),
        ("directory-home", "/home"),
        ("directory-refresh", "/home"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(control, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            picker.read_with(cx, |picker, cx| picker.value(cx)),
            Some(Directory::Remote(expected.into()))
        );
    }
}

#[gpui_kit::test]
fn unavailable_home_retains_a_supplied_editing_base_without_claiming_validation(
    cx: &mut TestAppContext,
) {
    use nocterm_session::testing::FsOperation;
    let source = Arc::new(InMemoryFs::default());
    block_on(source.create_dir("/project")).unwrap();
    source.fail_next(
        FsOperation::Home,
        "/",
        FsError::PermissionDenied { path: "/".into() },
    );
    let (handle, picker) = fixture(cx, source.clone());
    picker.update(cx, |picker, cx| {
        picker.set_fallback(Directory::Remote("/".into()), cx)
    });
    cx.run_until_parked();
    assert!(!picker.read_with(cx, |picker, cx| picker.is_valid(cx)));
    source.fail_next(
        FsOperation::Home,
        "/",
        FsError::PermissionDenied { path: "/".into() },
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-path-display", cx);
        window.input("/project", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        picker.read_with(cx, |picker, cx| picker.value(cx)),
        Some(Directory::Remote("/project".into()))
    );
}
