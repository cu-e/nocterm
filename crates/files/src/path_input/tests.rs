//! Native Input keyboard routing and async completion races.
use super::*;
use futures::{FutureExt as _, channel::oneshot};
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, px, test::TestWindowExt as _};
use nocterm_session::{DirEntry, FsError, FsFuture};
use std::sync::Mutex;

type Reply = oneshot::Sender<Result<Vec<DirEntry>, FsError>>;
#[derive(Default)]
struct PendingFs(Mutex<Vec<(String, Reply)>>);
impl RemoteFs for PendingFs {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        let (send, receive) = oneshot::channel();
        self.0.lock().unwrap().push((path.into(), send));
        async { receive.await.unwrap_or(Err(FsError::NotConnected)) }.boxed()
    }
}
impl PendingFs {
    fn take(&self, expected: &str) -> Reply {
        let (path, reply) = self.0.lock().unwrap().remove(0);
        assert_eq!(path, expected);
        reply
    }
}
fn entry(name: &str, directory: bool) -> DirEntry {
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
fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<PathInput>, Arc<PendingFs>) {
    let fs = Arc::new(PendingFs::default());
    let (window, input) = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| PathInput::new(window, cx))
        })
        .unwrap()
    });
    input.update(cx, |input, cx| {
        input.reset_remote(Some(fs.clone()), cx);
        input.accept(
            Directory::Remote("/work".into()),
            Some(Directory::Remote("/home/test".into())),
            cx,
        );
    });
    (window, input, fs)
}
fn start(value: &str, handle: AnyWindowHandle, input: &Entity<PathInput>, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        input.update(cx, |this, cx| this.edit(window, cx));
        window.render_frame(cx);
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
fn value(input: &Entity<PathInput>, cx: &mut TestAppContext) -> String {
    input.read_with(cx, |input, cx| input.input.read(cx).value().to_string())
}

#[gpui_kit::test]
fn native_tab_cycles_files_and_folders_and_enter_emits_only_navigation(cx: &mut TestAppContext) {
    let (window, input, fs) = fixture(cx);
    let navigations = Arc::new(Mutex::new(Vec::new()));
    let observed = navigations.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&input, move |_, event: &Navigate, _| {
            observed.lock().unwrap().push(event.0.clone())
        })
    });
    start("/etc/s", window, &input, cx);
    fs.take("/etc/")
        .send(Ok(vec![
            entry("ssh", true),
            entry("settings.json", false),
            entry("other", true),
        ]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/ssh/");
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/etc/settings.json");
    cx.update_window(window, |_, window, cx| {
        window.press("shift-tab", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(
        *navigations.lock().unwrap(),
        [Directory::Remote("/etc/ssh/".into())]
    );
    assert!(
        fs.0.lock().unwrap().is_empty(),
        "cycling uses the same cached directory"
    );
}

#[gpui_kit::test]
fn pending_tabs_are_applied_and_editing_discards_stale_remote_response(cx: &mut TestAppContext) {
    let (window, input, fs) = fixture(cx);
    start("/etc/", window, &input, cx);
    let stale = fs.take("/etc/");
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
        window.input("new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let _ = stale.send(Ok(vec![entry("stale", true)]));
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/new");
    assert!(input.read_with(cx, |input, _| input.cycle.is_none()));
    start("~/", window, &input, cx);
    let reply = fs.take("/home/test/");
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
        window.press("tab", cx);
    })
    .unwrap();
    reply
        .send(Ok(vec![
            entry("a", true),
            entry("b", false),
            entry("c", false),
        ]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "~/c");
}

#[gpui_kit::test]
fn error_preserves_draft_escape_cancels_and_session_reset_releases_completion(
    cx: &mut TestAppContext,
) {
    let (window, input, fs) = fixture(cx);
    start("missing", window, &input, cx);
    fs.take("/work")
        .send(Err(FsError::PermissionDenied {
            path: "/work".into(),
        }))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "missing");
    assert!(input.read_with(cx, |input, _| {
        input
            .error
            .as_ref()
            .is_some_and(|error| error.contains("permission denied"))
    }));
    cx.update_window(window, |_, window, cx| {
        window.press("escape", cx);
    })
    .unwrap();
    assert!(!input.read_with(cx, |input, _| input.editing));
    start("~/", window, &input, cx);
    let stale = fs.take("/home/test/");
    input.update(cx, |input, cx| input.reset_remote(None, cx));
    let _ = stale.send(Ok(vec![entry("old-host", true)]));
    cx.run_until_parked();
    assert!(input.read_with(cx, |input, _| input.directory.is_none()
        && input.cycle.is_none()));
}

#[gpui_kit::test]
fn native_local_completion_includes_hidden_names_unicode_and_files(cx: &mut TestAppContext) {
    let (window, input, _) = fixture(cx);
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("資料")).unwrap();
    std::fs::write(directory.path().join(".hidden file"), b"a").unwrap();
    input.update(cx, |input, cx| {
        input.accept(Directory::Local(directory.path().into()), None, cx)
    });
    start("", window, &input, cx);
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "資料/");
    cx.update_window(window, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), ".hidden file");
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
    fs.take("/etc/").send(Ok(vec![entry("ssh", true)])).unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    cx.run_until_parked();
    fs.take("/etc/ssh/")
        .send(Ok(vec![entry("config", false), entry("keys", true)]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("path-suggestion", 1usize), cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/etc/ssh/config");
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-z", cx);
    })
    .unwrap();
    assert!(
        input.read_with(cx, |input, _| input.cycle.is_none()),
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
        input.update(cx, |input, cx| input.edit(window, cx));
        window.input("/missing file", cx);
        window.press("enter", cx);
    })
    .unwrap();
    input.update(cx, |input, cx| {
        input.navigation_failed("Not a directory".into(), cx)
    });
    assert_eq!(value(&input, cx), "/missing file");
    assert!(input.read_with(cx, |input, _| input.editing && !input.navigating));
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
        assert!(input.read(cx).focus.is_focused(window));
        window.press("enter", cx);
        assert!(
            input.read(cx).editing,
            "Display remains keyboard-operable after navigation"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn directory_refresh_invalidates_cache_and_cancels_pending_completion(cx: &mut TestAppContext) {
    let (handle, input, fs) = fixture(cx);
    start("/etc/", handle, &input, cx);
    fs.take("/etc/")
        .send(Ok(vec![entry("old", false)]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/old");
    input.update(cx, |input, cx| {
        input.begin_navigation(cx);
        input.accept(Directory::Remote("/work".into()), None, cx);
    });
    start("/etc/", handle, &input, cx);
    let pending = fs.take("/etc/");
    input.update(cx, |input, cx| {
        input.begin_navigation(cx);
        input.accept(Directory::Remote("/new".into()), None, cx);
    });
    let _ = pending.send(Ok(vec![entry("late", false)]));
    cx.run_until_parked();
    assert!(input.read_with(cx, |input, _| input.cycle.is_none()
        && input.cache.is_none()));
    start("/etc/", handle, &input, cx);
    fs.take("/etc/")
        .send(Ok(vec![entry("fresh", false)]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/fresh");
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
        fs.take("/etc/")
            .send(Ok((0..30)
                .map(|index| entry(&format!("folder-{index:02}"), false))
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
            assert!(input.read(cx).focus.is_focused(window));
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
            !input.read(cx).editing,
            "Do not expose a lossy editable filename"
        );
        assert_eq!(input.read(cx).directory, Some(Directory::Local(directory)));
        assert!(
            input
                .read(cx)
                .encoding_error
                .as_ref()
                .unwrap()
                .contains("cannot be represented")
        );
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
            input.read(cx).editing,
            "Representable paths remain editable"
        );
    })
    .unwrap();
}
