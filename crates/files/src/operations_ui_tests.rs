//! Native context menus and mutation dialogs; production remains read-only here.
use super::*;
use gpui_kit::{InputEvent as _, component::WindowExt as _, test::TestWindowExt as _};
use nocterm_session::{FileMetadata, FsCapabilities};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

const RENAME: usize = 0;
const COPY_NAME: usize = 2;
const COPY_PATH: usize = 3;
const DELETE: usize = 5;
const PROPERTIES: usize = 7;

#[derive(Default)]
struct OperationsFs {
    calls: Mutex<Vec<String>>,
    mode: AtomicU32,
    read_only: bool,
    no_capabilities: bool,
    symlink: bool,
    metadata_disconnected: bool,
    remove_gate: Mutex<Option<oneshot::Receiver<()>>>,
    rename_gate: Mutex<Option<oneshot::Receiver<()>>>,
    rename_done: Arc<AtomicBool>,
}
impl OperationsFs {
    fn writable() -> Self {
        Self {
            mode: AtomicU32::new(0o4640),
            ..Self::default()
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}
impl RemoteFs for OperationsFs {
    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            metadata: !self.no_capabilities,
            rename: !self.no_capabilities && !self.read_only,
            remove: !self.no_capabilities && !self.read_only,
            set_permissions: !self.no_capabilities && !self.read_only,
        }
    }
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        self.calls.lock().unwrap().push(format!("list:{path}"));
        let names = if path.ends_with("/folder") {
            vec!["one", "two"]
        } else {
            vec!["a.txt", "b.txt"]
        };
        let mut entries = names
            .into_iter()
            .map(|name| entry(name, EntryKind::File))
            .collect::<Vec<_>>();
        if path == "/home/test" {
            entries.push(entry("folder", EntryKind::Directory));
        }
        async { Ok(entries) }.boxed()
    }
    fn metadata(&self, path: &str) -> FsFuture<FileMetadata> {
        self.calls.lock().unwrap().push(format!("metadata:{path}"));
        if self.metadata_disconnected {
            return async { Err(FsError::NotConnected) }.boxed();
        }
        let metadata = FileMetadata {
            kind: if path.ends_with("/folder") {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            is_symlink: self.symlink,
            permissions: Some(self.mode.load(Ordering::Acquire)),
            size: Some(42),
            uid: Some(1000),
            gid: Some(1000),
            modified: Some(1_700_000_000),
        };
        async { Ok(metadata) }.boxed()
    }
    fn rename(&self, old: &str, new: &str) -> FsFuture<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("rename:{old}->{new}"));
        let gate = self.rename_gate.lock().unwrap().take();
        let done = self.rename_done.clone();
        async move {
            if let Some(gate) = gate {
                gate.await.unwrap();
            }
            done.store(true, Ordering::Release);
            Ok(())
        }
        .boxed()
    }
    fn set_permissions(&self, path: &str, mode: u32) -> FsFuture<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("permissions:{path}:{mode:04o}"));
        self.mode.store(mode, Ordering::Release);
        async { Ok(()) }.boxed()
    }
    fn remove_file(&self, path: &str) -> FsFuture<()> {
        self.calls.lock().unwrap().push(format!("unlink:{path}"));
        let gate = self.remove_gate.lock().unwrap().take();
        async move {
            if let Some(gate) = gate {
                gate.await.unwrap();
            }
            Ok(())
        }
        .boxed()
    }
    fn remove_dir(&self, path: &str) -> FsFuture<()> {
        self.calls.lock().unwrap().push(format!("rmdir:{path}"));
        async { Ok(()) }.boxed()
    }
}

fn visible_panel(
    cx: &mut TestAppContext,
    fs: Arc<dyn RemoteFs>,
    local: &Path,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<SessionItem>,
    Entity<FilesPanel>,
) {
    let (handle, workspace, item, panel) = panel_fixture(cx, fs);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        cx.set_reduce_motion(true);
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
        panel.update(cx, |panel, cx| panel.load_local(local.into(), cx));
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1100.)));
    cx.run_until_parked();
    (handle, workspace, item, panel)
}
fn row_point(window: &Window, local: bool, ix: usize) -> gpui_kit::Point<gpui_kit::Pixels> {
    let toolbar = window
        .find(if local { "local-home" } else { "files-home" })
        .bounds();
    gpui_kit::point(
        toolbar.center().x,
        toolbar.bottom() + px(36. + ix as f32 * 32.),
    )
}
fn context_menu(cx: &mut TestAppContext, handle: AnyWindowHandle, local: bool, ix: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let position = row_point(window, local, ix);
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                position,
                button: MouseButton::Right,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                position,
                button: MouseButton::Right,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}
fn choose(cx: &mut TestAppContext, handle: AnyWindowHandle, ix: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(ix, cx);
    })
    .unwrap();
    cx.run_until_parked();
}
fn button(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}
fn clipboard(cx: &TestAppContext) -> String {
    cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
}

#[gpui_kit::test(iterations = 20)]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn dismissed_pending_rename_cannot_close_a_new_dialog(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (release, gate) = oneshot::channel();
    let fs = Arc::new(OperationsFs {
        rename_gate: Mutex::new(Some(gate)),
        ..OperationsFs::writable()
    });
    let (handle, _, _, _) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    choose(cx, handle, RENAME);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.input("renamed.txt", cx);
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert!(
        fs.calls()
            .contains(&"rename:/home/test/a.txt->/home/test/renamed.txt".into())
    );
    assert!(!fs.rename_done.load(Ordering::Acquire));

    let editor = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let position = window.find("file-dialog-close").bounds().center();
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui_kit::MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                gpui_kit::MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            assert!(!window.has_active_dialog(cx));
            let editor = cx.new(|cx| {
                gpui_kit::component::input::InputState::new(window, cx)
                    .default_value("unsaved replacement draft")
            });
            let draft = editor.clone();
            window.open_dialog(cx, move |dialog, _, _| {
                dialog
                    .title("Replacement editor")
                    .child(gpui_kit::component::input::Input::new(&draft))
                    .child(
                        gpui_kit::component::button::Button::new("replacement-dialog-marker")
                            .label("Save draft"),
                    )
            });
            assert!(window.has_active_dialog(cx));
            // Keep the previous native frame until completion to exercise
            // normal render-cache lifetimes between consecutive modal events.
            assert!(!fs.rename_done.load(Ordering::Acquire));
            release.send(()).unwrap();
            editor
        })
        .unwrap();
    cx.run_until_parked();
    assert!(fs.rename_done.load(Ordering::Acquire));
    cx.update_window(handle, |_, window, cx| {
        assert!(
            window.has_active_dialog(cx),
            "a dismissed rename completion must not close the replacement dialog"
        );
        window.render_frame(cx);
        assert!(window.try_find("replacement-dialog-marker").is_some());
        assert_eq!(editor.read(cx).value(), "unsaved replacement draft");
    })
    .unwrap();
}

#[gpui_kit::test]
fn local_context_copy_and_collision_rename_preserve_existing_files(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("a.txt"), b"source").unwrap();
    std::fs::write(directory.path().join("b.txt"), b"destination").unwrap();
    let (handle, _, _, _) = visible_panel(cx, Arc::new(OperationsFs::writable()), directory.path());
    context_menu(cx, handle, true, 0);
    choose(cx, handle, COPY_NAME);
    assert_eq!(clipboard(cx), "a.txt");
    context_menu(cx, handle, true, 0);
    choose(cx, handle, COPY_PATH);
    assert_eq!(
        clipboard(cx),
        directory.path().join("a.txt").display().to_string()
    );
    context_menu(cx, handle, true, 0);
    choose(cx, handle, RENAME);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.has_focused_input(cx),
            "rename dialog must focus its text input; popup still open: {}; inputs: {:?}",
            window.try_find("popup-menu").is_some(),
            gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .filter(|item| item.value().is_some())
                .collect::<Vec<_>>()
        );
        window.press("ctrl-a", cx);
        window.input("bad/name", cx);
        window.render_frame(cx);
        // GPUI exposes no aria-disabled setter. Its real disabled Button drops
        // native focus capability; inspect that rather than inventing UI state.
        assert_eq!(window.find("file-dialog-submit").focused(), None);
        window.click("file-dialog-submit", cx);
        window.render_frame(cx);
        assert!(window.try_find("file-dialog-submit").is_some());
        assert!(directory.path().join("a.txt").exists());
        window.press("ctrl-a", cx);
        window.input("b.txt", cx);
        window.render_frame(cx);
        assert!(window.find("file-dialog-submit").focused().is_some());
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert_eq!(
        std::fs::read(directory.path().join("a.txt")).unwrap(),
        b"source"
    );
    assert_eq!(
        std::fs::read(directory.path().join("b.txt")).unwrap(),
        b"destination"
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("file-dialog-submit").is_some(),
            "collision keeps the rename dialog open"
        );
        window.press("ctrl-a", cx);
        window.input("renamed.txt", cx);
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert!(!directory.path().join("a.txt").exists());
    assert_eq!(
        std::fs::read(directory.path().join("renamed.txt")).unwrap(),
        b"source"
    );
}

#[gpui_kit::test]
fn local_nonempty_delete_requires_confirmation_and_preserves_symlink_target(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let folder = directory.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("inside"), b"inside").unwrap();
    std::fs::write(outside.path().join("kept"), b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), folder.join("link")).unwrap();
    let (handle, _, _, _) = visible_panel(cx, Arc::new(OperationsFs::writable()), directory.path());
    context_menu(cx, handle, true, 0);
    choose(cx, handle, DELETE);
    assert!(
        folder.join("inside").exists(),
        "opening confirmation must not delete"
    );
    button(cx, handle, "file-dialog-close");
    assert!(folder.join("inside").exists());
    context_menu(cx, handle, true, 0);
    choose(cx, handle, DELETE);
    button(cx, handle, "file-dialog-submit");
    assert!(!folder.exists());
    assert_eq!(
        std::fs::read(outside.path().join("kept")).unwrap(),
        b"outside"
    );
}

#[gpui_kit::test]
fn remote_context_copy_and_permissions_apply_preserve_special_bits(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(OperationsFs::writable());
    let (handle, _, _, _) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    choose(cx, handle, COPY_NAME);
    assert_eq!(clipboard(cx), "a.txt");
    context_menu(cx, handle, false, 1);
    choose(cx, handle, COPY_PATH);
    assert_eq!(clipboard(cx), "/home/test/a.txt");
    context_menu(cx, handle, false, 1);
    choose(cx, handle, PROPERTIES);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("permission-Owner-Read").checked(), Some(true));
        assert_eq!(
            window.find("permission-Others-Execute").checked(),
            Some(false)
        );
        window.click("file-dialog-submit", cx);
        assert!(
            window.try_find("popup-menu").is_none(),
            "Properties must dismiss the old context menu"
        );
        assert!(
            window.find("permission-Others-Execute").visible(),
            "checkbox visible"
        );
        window.click("permission-Others-Execute", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("permission-Others-Execute").checked(),
            Some(true),
            "checkbox must reflect mode edit before Apply"
        );
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert!(
        fs.calls()
            .contains(&"permissions:/home/test/a.txt:4641".into())
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("permission-Others-Execute").checked(),
            Some(true)
        );
        window.click("file-dialog-submit", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn metadata_only_filesystem_properties_cannot_mutate(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(OperationsFs {
        read_only: true,
        ..OperationsFs::writable()
    });
    let (handle, _, _, _) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(RENAME, cx);
        window.render_frame(cx);
        assert!(window.try_find("file-dialog-submit").is_none());
        window.within("popup-menu").click(DELETE, cx);
        window.render_frame(cx);
        assert!(window.try_find("file-dialog-submit").is_none());
    })
    .unwrap();
    choose(cx, handle, PROPERTIES);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("permission-Owner-Read").checked(), Some(true));
        window.click("permission-Owner-Read", cx);
        window.click("file-dialog-submit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        fs.calls()
            .iter()
            .all(|call| !call.starts_with("permissions:"))
    );
}

#[gpui_kit::test]
fn selected_remote_context_preserves_multi_selection_and_busy_stop_blocks_next_delete(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (send, receive) = oneshot::channel();
    let fs = Arc::new(OperationsFs {
        remove_gate: Mutex::new(Some(receive)),
        ..OperationsFs::writable()
    });
    let (handle, _, _, panel) = visible_panel(cx, fs.clone(), directory.path());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        pointer_click(window, row_point(window, false, 1), Default::default(), cx);
        pointer_click(
            window,
            row_point(window, false, 2),
            gpui_kit::Modifiers {
                control: true,
                ..Default::default()
            },
            cx,
        );
    })
    .unwrap();
    context_menu(cx, handle, false, 1);
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.remote_selected.iter().copied().collect::<Vec<_>>(),
            [1, 2]
        )
    });
    choose(cx, handle, DELETE);
    assert!(!fs.calls().iter().any(|call| call.starts_with("unlink:")));
    button(cx, handle, "file-dialog-submit");
    assert!(fs.calls().contains(&"unlink:/home/test/a.txt".into()));
    button(cx, handle, "file-dialog-close");
    send.send(()).unwrap();
    cx.run_until_parked();
    assert!(!fs.calls().contains(&"unlink:/home/test/b.txt".into()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("file-dialog-close").is_some());
    })
    .unwrap();
    button(cx, handle, "file-dialog-close");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("file-dialog-close").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn filesystem_without_capabilities_keeps_copy_actions_and_refuses_all_dialogs(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(OperationsFs {
        no_capabilities: true,
        ..OperationsFs::writable()
    });
    let (handle, _, _, _) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    for ix in [RENAME, DELETE, PROPERTIES] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").click(ix, cx);
            window.render_frame(cx);
            assert!(window.try_find("file-dialog-submit").is_none());
        })
        .unwrap();
    }
    choose(cx, handle, COPY_PATH);
    assert_eq!(clipboard(cx), "/home/test/a.txt");
    assert!(fs.calls().iter().all(|call| call.starts_with("list:")));
}

#[gpui_kit::test]
fn symlink_properties_stay_read_only_even_on_mutable_filesystem(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(OperationsFs {
        symlink: true,
        ..OperationsFs::writable()
    });
    let (handle, _, _, _) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    choose(cx, handle, PROPERTIES);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("permission-Owner-Read", cx);
        window.render_frame(cx);
        assert_eq!(window.find("permission-Owner-Read").checked(), Some(true));
        window.click("file-dialog-submit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        fs.calls()
            .iter()
            .all(|call| !call.starts_with("permissions:"))
    );
}

#[gpui_kit::test]
fn remote_recursive_delete_is_confirmed_and_removes_children_before_folder(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(OperationsFs::writable());
    let (handle, _, _, _) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 0);
    choose(cx, handle, DELETE);
    assert!(
        fs.calls()
            .iter()
            .all(|call| !call.starts_with("unlink:") && !call.starts_with("rmdir:"))
    );
    button(cx, handle, "file-dialog-submit");
    let removals = fs
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("unlink:") || call.starts_with("rmdir:"))
        .collect::<Vec<_>>();
    assert_eq!(
        removals,
        [
            "unlink:/home/test/folder/one",
            "unlink:/home/test/folder/two",
            "rmdir:/home/test/folder"
        ]
    );
}

#[gpui_kit::test]
fn disconnected_metadata_shows_retry_and_never_enables_permissions(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(OperationsFs {
        metadata_disconnected: true,
        ..OperationsFs::writable()
    });
    let (handle, _, _, _) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    choose(cx, handle, PROPERTIES);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("retry-properties").is_some());
        assert!(window.try_find("permission-Owner-Read").is_none());
        window.click("file-dialog-submit", cx);
        window.click("retry-properties", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        fs.calls()
            .iter()
            .filter(|call| call.as_str() == "metadata:/home/test/a.txt")
            .count(),
        2
    );
    assert!(
        fs.calls()
            .iter()
            .all(|call| !call.starts_with("permissions:"))
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn local_properties_apply_and_right_click_keep_multi_selection(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    let b = directory.path().join("b.txt");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();
    std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o2640)).unwrap();
    let (handle, _, _, panel) =
        visible_panel(cx, Arc::new(OperationsFs::writable()), directory.path());
    context_menu(cx, handle, true, 0);
    choose(cx, handle, PROPERTIES);
    button(cx, handle, "permission-Others-Execute");
    button(cx, handle, "file-dialog-submit");
    assert_eq!(
        std::fs::metadata(&a).unwrap().permissions().mode() & 0o7777,
        0o2641
    );
    button(cx, handle, "file-dialog-close");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        pointer_click(window, row_point(window, true, 0), Default::default(), cx);
        pointer_click(
            window,
            row_point(window, true, 1),
            gpui_kit::Modifiers {
                control: true,
                ..Default::default()
            },
            cx,
        );
    })
    .unwrap();
    context_menu(cx, handle, true, 0);
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.local.selected.iter().copied().collect::<Vec<_>>(),
            [0, 1]
        )
    });
    choose(cx, handle, DELETE);
    assert!(a.exists() && b.exists());
    button(cx, handle, "file-dialog-submit");
    assert!(!a.exists() && !b.exists());
}

#[path = "rename_ui_tests.rs"]
mod rename;
