//! Native Input keyboard routing and async completion races.
use super::*;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, px, test::TestWindowExt as _};
use nocterm_explorer::{directory::valid_name, testing::MemoryDirectoryReader};
use nocterm_session::{
    DirEntry, FsError,
    testing::{InMemoryFs, PendingListing},
};
use nocterm_session::{EntryKind, local_fs::LocalFs};
use std::sync::Mutex;

pub(super) fn take(fs: &InMemoryFs, expected: &str) -> PendingListing {
    let reply = fs.take_pending_listing().expect("a pending listing");
    assert_eq!(reply.path(), expected);
    reply
}
pub(super) fn entry(name: &str, directory: bool) -> DirEntry {
    DirEntry {
        name: name.into(),
        kind: if directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        is_symlink: false,
        size: None,
    }
}
pub(super) fn native_test_path(name: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::PathBuf::from("C:\\").join(name)
    } else {
        std::path::PathBuf::from("/").join(name)
    }
}
pub(super) fn fixture(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<PathInput>, Arc<InMemoryFs>) {
    fixture_with_local(cx, Arc::new(InMemoryFs::default()))
}
pub(super) fn fixture_with_local(
    cx: &mut TestAppContext,
    local: Arc<dyn LocalFs>,
) -> (AnyWindowHandle, Entity<PathInput>, Arc<InMemoryFs>) {
    let fs = Arc::new(InMemoryFs::default());
    fs.script_listings();
    let (window, input) = cx.update(|cx| {
        gpui_kit::init(cx);
        install_bindings(cx);
        crate::init(
            crate::DesignTokens::builtin(),
            (nocterm_store::SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| PathInput::new(Arc::new(MemoryDirectoryReader::local(local)), window, cx))
        })
        .unwrap()
    });
    input.update(cx, |input, cx| {
        input.replace_reader(
            Some(Arc::new(MemoryDirectoryReader::remote(fs.clone()))),
            cx,
        );
        input.accept(
            Directory::Remote("/work".into()),
            Some(Directory::Remote("/home/test".into())),
            cx,
        );
    });
    (window, input, fs)
}
pub(super) fn start(
    value: &str,
    handle: AnyWindowHandle,
    _input: &Entity<PathInput>,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        if window.try_find("explorer-path-display").is_some() {
            window.click("explorer-path-display", cx);
        } else {
            window.press("ctrl-a", cx);
        }
        if value.is_empty() {
            window.press("backspace", cx);
        } else {
            window.input(value, cx);
        }
        window.press("tab", cx);
    })
    .unwrap();
    cx.run_until_parked();
}
pub(super) fn value(_input: &Entity<PathInput>, cx: &mut TestAppContext) -> String {
    observed(cx, |window| {
        window
            .find("explorer-path-editor")
            .value()
            .unwrap()
            .to_owned()
    })
}
pub(super) fn observed<T>(cx: &mut TestAppContext, observe: impl FnOnce(&Window) -> T) -> T {
    let handle = cx.update(|cx| cx.windows()[0]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        observe(window)
    })
    .unwrap()
}
pub(super) fn visible(cx: &mut TestAppContext, id: &'static str) -> bool {
    observed(cx, |window| {
        if id == "path-completion-popup" {
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|element| {
                    element.label() == Some("Directory suggestions") && element.visible()
                })
        } else {
            window.try_find(id).is_some_and(|element| element.visible())
        }
    })
}
pub(super) fn suggestions(cx: &mut TestAppContext) -> Vec<String> {
    observed(cx, |window| {
        (0usize..100)
            .map_while(|index| window.try_find(("path-suggestion", index)))
            .map(|element| element.label().unwrap().to_owned())
            .collect()
    })
}

#[gpui_kit::test]
fn native_tab_excludes_files_cycles_folders_and_enter_emits_only_navigation(
    cx: &mut TestAppContext,
) {
    let (window, input, fs) = fixture(cx);
    let navigations = Arc::new(Mutex::new(Vec::new()));
    let observed = navigations.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&input, move |_, event: &Navigate, _| {
            observed.lock().unwrap().push(event.directory.clone())
        })
    });
    start("/etc/s", window, &input, cx);
    take(&fs, "/etc/")
        .send(Ok(vec![
            entry("ssh", true),
            entry("settings", true),
            entry("settings.json", false),
            entry("other", true),
        ]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/settings/");
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/etc/ssh/");
    cx.update_window(window, |_, window, cx| {
        window.press("shift-tab", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(
        *navigations.lock().unwrap(),
        [Directory::Remote("/etc/settings/".into())]
    );
    assert!(
        !fs.has_pending_listings(),
        "cycling uses the same cached directory"
    );
}

#[gpui_kit::test]
fn pending_tabs_are_applied_and_editing_discards_stale_remote_response(cx: &mut TestAppContext) {
    let (window, input, fs) = fixture(cx);
    start("/etc/", window, &input, cx);
    let stale = take(&fs, "/etc/");
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
        window.input("new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let _ = stale.send(Ok(vec![entry("stale", true)]));
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/new");
    assert!(!visible(cx, "path-completion-popup"));
    start("~/", window, &input, cx);
    let reply = take(&fs, "/home/test/");
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
        window.press("tab", cx);
    })
    .unwrap();
    reply
        .send(Ok(vec![
            entry("a", true),
            entry("b", true),
            entry("c", true),
        ]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "~/c/");
}

#[gpui_kit::test]
fn error_preserves_draft_escape_cancels_and_session_reset_releases_completion(
    cx: &mut TestAppContext,
) {
    let (window, input, fs) = fixture(cx);
    start("missing", window, &input, cx);
    take(&fs, "/work")
        .send(Err(FsError::PermissionDenied {
            path: "/work".into(),
        }))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "missing");
    assert!(observed(cx, |window| window
        .find("explorer-path-error")
        .label()
        .unwrap()
        .contains("permission denied")));
    cx.update_window(window, |_, window, cx| {
        window.press("escape", cx);
    })
    .unwrap();
    assert!(visible(cx, "explorer-path-display"));
    start("~/", window, &input, cx);
    let stale = take(&fs, "/home/test/");
    input.update(cx, |input, cx| input.replace_reader(None, cx));
    let _ = stale.send(Ok(vec![entry("old-host", true)]));
    cx.run_until_parked();
    assert!(!visible(cx, "explorer-path-display") && !visible(cx, "explorer-path-editor"));
}

#[gpui_kit::test]
fn native_local_completion_includes_hidden_unicode_directories_and_excludes_files(
    cx: &mut TestAppContext,
) {
    let directory = native_test_path("completion-test");
    let local = InMemoryFs::default();
    local.seed_native_directory(&directory).unwrap();
    local
        .seed_native_directory(&directory.join("資料"))
        .unwrap();
    local
        .seed_native_directory(&directory.join(".hidden folder"))
        .unwrap();
    local
        .seed_native_file(&directory.join(".hidden file"), b"a".to_vec())
        .unwrap();
    let local = Arc::new(local);
    let (window, input, _) = fixture_with_local(cx, local.clone());
    input.update(cx, |input, cx| {
        input.replace_reader(Some(Arc::new(MemoryDirectoryReader::local(local))), cx)
    });
    input.update(cx, |input, cx| {
        input.accept(Directory::Local(directory.clone()), None, cx)
    });
    start("", window, &input, cx);
    cx.run_until_parked();
    assert_eq!(value(&input, cx), ".hidden folder/");
    assert_eq!(suggestions(cx).len(), 2);
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "資料/");
}

#[gpui_kit::test]
fn path_and_suggestion_are_clickable_and_single_folder_tab_descends(cx: &mut TestAppContext) {
    let (handle, input, fs) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-path-display", cx);
        window.render_frame(cx);
        window.input("/etc/ss", cx);
        window.press("tab", cx);
    })
    .unwrap();
    cx.run_until_parked();
    take(&fs, "/etc/")
        .send(Ok(vec![entry("ssh", true)]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    cx.run_until_parked();
    take(&fs, "/etc/ssh/")
        .send(Ok(vec![
            entry("config", false),
            entry("keys", true),
            entry("logs", true),
        ]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("path-suggestion", 1usize), cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/etc/ssh/logs/");
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-z", cx);
    })
    .unwrap();
    assert!(
        !visible(cx, "path-completion-popup"),
        "Undo is a native edit that restarts completion"
    );
}

#[test]
fn completion_ignores_unrepresentable_or_protocol_invalid_names() {
    for name in ["", ".", "..", "x/y", "x\0y", "x\ny", "x\x1by"] {
        assert!(!valid_name(name, false));
    }
    assert!(valid_name(".hidden 資料's", false));
    assert!(
        valid_name("x\\y", false),
        "remote backslashes are literal POSIX names"
    );
}

#[gpui_kit::test]
fn navigation_failure_keeps_draft_and_success_restores_display_focus(cx: &mut TestAppContext) {
    let (handle, input, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-path-display", cx);
        window.input("/missing file", cx);
        window.press("enter", cx);
    })
    .unwrap();
    input.update(cx, |input, cx| {
        input.navigation_failed("Not a directory".into(), cx)
    });
    assert_eq!(value(&input, cx), "/missing file");
    assert!(visible(cx, "explorer-path-editor"));
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("/valid folder", cx);
        window.press("enter", cx);
    })
    .unwrap();
    input.update(cx, |input, cx| {
        input.accept(Directory::Remote("/valid folder".into()), None, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.find("explorer-path-display").focused(), Some(true));
        window.press("enter", cx);
        assert!(
            window
                .try_find("explorer-path-editor")
                .is_some_and(|editor| editor.visible()),
            "Display remains keyboard-operable after navigation"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn directory_refresh_invalidates_cache_and_cancels_pending_completion(cx: &mut TestAppContext) {
    let (handle, input, fs) = fixture(cx);
    start("/etc/", handle, &input, cx);
    take(&fs, "/etc/")
        .send(Ok(vec![entry("old", true)]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/old/");
    input.update(cx, |input, cx| {
        input.begin_navigation_with_mode(NavigationMode::CloseEditor, cx);
        input.accept(Directory::Remote("/work".into()), None, cx);
    });
    start("/etc/", handle, &input, cx);
    let pending = take(&fs, "/etc/");
    input.update(cx, |input, cx| {
        input.begin_navigation_with_mode(NavigationMode::CloseEditor, cx);
        input.accept(Directory::Remote("/new".into()), None, cx);
    });
    let _ = pending.send(Ok(vec![entry("late", true)]));
    cx.run_until_parked();
    assert!(!visible(cx, "path-completion-popup"));
    start("/etc/", handle, &input, cx);
    take(&fs, "/etc/")
        .send(Ok(vec![entry("fresh", true)]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/fresh/");
}

#[gpui_kit::test]
fn narrow_light_and_dark_location_keeps_selected_suggestion_visible(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeMode};
    let (handle, input, fs) = fixture(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(240.), px(260.)));
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update_window(handle, |_, _, cx| Theme::change(mode, None, cx))
            .unwrap();
        input.update(cx, |input, cx| {
            input.accept(Directory::Remote("/work".into()), None, cx)
        });
        start("/etc/", handle, &input, cx);
        take(&fs, "/etc/")
            .send(Ok((0..30)
                .map(|index| entry(&format!("folder-{index:02}"), true))
                .collect()))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            for _ in 0..24 {
                window.press("tab", cx);
                window.render_frame(cx);
            }
            let selected = window.find(("path-suggestion", 24usize));
            assert!(
                selected.visible(),
                "Tab-selected row must remain visible after scrolling"
            );
            let bounds = selected.bounds();
            assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
            assert!(bounds.origin.y >= px(0.));
            assert!(bounds.bottom() <= window.viewport_size().height);
            window.click(("path-suggestion", 24usize), cx);
            window.press("escape", cx);
            assert_eq!(window.find("explorer-path-display").focused(), Some(true));
        })
        .unwrap();
    }
}

#[cfg(unix)]
#[gpui_kit::test]
fn clicking_non_utf8_directory_shows_readonly_diagnostic_without_lossy_draft(
    cx: &mut TestAppContext,
) {
    use std::os::unix::ffi::OsStringExt as _;
    let (handle, input, _) = fixture(cx);
    let directory = std::path::PathBuf::from(std::ffi::OsString::from_vec(
        b"/tmp/directory-\xff".to_vec(),
    ));
    input.update(cx, |input, cx| {
        input.accept(Directory::Local(directory.clone()), None, cx)
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-path-display", cx);
        window.render_frame(cx);
        let error = window.find("explorer-path-error");
        assert!(
            error.visible(),
            "An uneditable directory must explain why clicking failed"
        );
        assert!(
            window.try_find("explorer-path-editor").is_none(),
            "Do not expose a lossy editable filename"
        );
        assert_eq!(
            window.find("explorer-path-display").value(),
            Some(directory.to_string_lossy().as_ref())
        );
        assert!(error.label().unwrap().contains("cannot be represented"));
    })
    .unwrap();
    input.update(cx, |input, cx| {
        input.accept(Directory::Local("/tmp/readable".into()), None, cx)
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("explorer-path-error").is_none(),
            "Successful listing clears obsolete diagnostic"
        );
        window.click("explorer-path-display", cx);
        assert!(
            window
                .try_find("explorer-path-editor")
                .is_some_and(|editor| editor.visible()),
            "Representable paths remain editable"
        );
    })
    .unwrap();
}
