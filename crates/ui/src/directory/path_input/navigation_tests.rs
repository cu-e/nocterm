//! Directory-only completion and native keyboard/navigation lifecycle.
use super::{tests::*, *};
use gpui_kit::{TestAppContext, px, test::TestWindowExt as _};
use nocterm_explorer::testing::MemoryDirectoryReader;
use nocterm_session::{EntryKind, testing::InMemoryFs};

struct EditorAndOther {
    location: Entity<PathInput>,
    other: Entity<InputState>,
}
impl Render for EditorAndOther {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .gap_2()
            .child(self.location.clone())
            .child(Input::new(&self.other).small())
    }
}
fn other_input(
    handle: gpui_kit::AnyWindowHandle,
    location: &Entity<PathInput>,
    cx: &mut TestAppContext,
) -> Entity<InputState> {
    cx.update_window(handle, |_, window, cx| {
        let other = cx.new(|cx| InputState::new(window, cx).default_value("/work"));
        window.replace_root(cx, |_, _| EditorAndOther {
            location: location.clone(),
            other: other.clone(),
        });
        window.activate_window();
        window.render_frame(cx);
        other
    })
    .unwrap()
}

#[gpui_kit::test]
fn pending_arrows_wrap_and_single_folder_arrows_never_descend(cx: &mut TestAppContext) {
    let (handle, input, fs) = fixture(cx);
    start("/etc/", handle, &input, cx);
    let reply = take(&fs, "/etc/");
    cx.update_window(handle, |_, window, cx| {
        window.press("up", cx);
        window.press("up", cx);
        window.press("down", cx);
    })
    .unwrap();
    reply
        .send(Ok(vec![
            entry("a", true),
            entry("b", true),
            entry("file", false),
        ]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/etc/b/");
    cx.update_window(handle, |_, window, cx| {
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("path-suggestion", 0usize)).selected(),
            Some(true)
        );
        assert_eq!(window.find("explorer-path-editor").value(), Some("/etc/a/"));
        window.press("up", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/etc/b/");
    start("/single/", handle, &input, cx);
    take(&fs, "/single/")
        .send(Ok(vec![entry("only", true)]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("up", cx);
        window.press("down", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/single/only/");
    assert!(!fs.has_pending_listings());
}

#[gpui_kit::test]
fn popup_enter_keeps_absolute_unicode_value_caret_then_plain_enter_closes(cx: &mut TestAppContext) {
    let (handle, input, fs) = fixture(cx);
    start("~/", handle, &input, cx);
    take(&fs, "/home/test/")
        .send(Ok(vec![entry("資料 folder", true)]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    assert!(!visible(cx, "path-completion-popup"));
    input.update(cx, |input, cx| {
        input.accept(Directory::Remote("/home/test/資料 folder".into()), None, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("explorer-path-editor").focused(), Some(true));
        assert_eq!(
            window.find("explorer-path-editor").value(),
            Some("/home/test/資料 folder")
        );
        assert!(
            window
                .try_find(("path-completion-popup", input.entity_id()))
                .is_none()
        );
        window.input("/next", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/home/test/資料 folder/next");
    assert!(!visible(cx, "path-completion-popup"));
    input.update(cx, |input, cx| {
        input.accept(
            Directory::Remote("/home/test/資料 folder/next".into()),
            None,
            cx,
        )
    });
    cx.run_until_parked();
    assert!(visible(cx, "explorer-path-display"));
}

#[gpui_kit::test]
fn pending_enter_escape_and_blur_do_not_reopen_or_steal_focus(cx: &mut TestAppContext) {
    let (handle, input, fs) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .unwrap();
    for escape in [false, true] {
        start("/pending/", handle, &input, cx);
        let reply = take(&fs, "/pending/");
        cx.update_window(handle, |_, window, cx| {
            window.press("enter", cx);
            window.render_frame(cx);
            if escape {
                window.press("escape", cx);
            } else {
                window.blur(cx);
            }
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let _ = reply.send(Ok(vec![entry("late", true)]));
        input.update(cx, |input, cx| {
            input.accept(Directory::Remote("/pending/".into()), None, cx)
        });
        cx.run_until_parked();
        assert!(
            visible(cx, "explorer-path-display") && !visible(cx, "path-completion-popup"),
            "escape={escape}"
        );
        cx.update_window(handle, |_, window, cx| {
            if !escape {
                window.render_frame(cx);
                assert_eq!(window.find("explorer-path-display").focused(), Some(false));
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn display_and_input_share_text_metrics_and_do_not_resize_on_navigation(cx: &mut TestAppContext) {
    let (handle, input, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.set_rem_size(px(20.));
        window.render_frame(cx);
        let display = window.find("explorer-path-display").bounds();
        window.click("explorer-path-display", cx);
        window.render_frame(cx);
        let editor = window.find("explorer-path-editor");
        let frame = editor.bounds();
        assert_eq!(display.size, frame.size);
        assert_eq!(frame.size.height, px(28.));
        assert_eq!(editor.value(), Some("/work"));
        input.update(cx, |input, cx| {
            input.begin_navigation_with_mode(NavigationMode::KeepEditing, cx)
        });
        window.render_frame(cx);
        assert_eq!(window.find("explorer-path-editor").bounds(), frame);
        assert_eq!(window.find("explorer-path-editor").focused(), Some(true));
    })
    .unwrap();
}

#[cfg(unix)]
#[gpui_kit::test]
fn local_only_directory_symlinks_are_suggested(cx: &mut TestAppContext) {
    let root = std::path::PathBuf::from("/completion-links");
    let local = InMemoryFs::default();
    local.seed_native_directory(&root).unwrap();
    local
        .seed_native_directory(&root.join("directory"))
        .unwrap();
    local
        .seed_native_file(&root.join("file"), b"text".to_vec())
        .unwrap();
    local
        .seed_native_alias(&root.join("dir-link"), &root.join("directory"))
        .unwrap();
    local
        .seed_native_alias(&root.join("file-link"), &root.join("file"))
        .unwrap();
    local
        .seed_native_alias(&root.join("broken-link"), &root.join("missing"))
        .unwrap();
    let local = Arc::new(local);
    let (handle, input, _) = fixture_with_local(cx, local.clone());
    input.update(cx, |input, cx| {
        input.replace_reader(Some(Arc::new(MemoryDirectoryReader::local(local))), cx)
    });
    input.update(cx, |input, cx| {
        input.accept(Directory::Local(root), None, cx)
    });
    start("", handle, &input, cx);
    cx.run_until_parked();
    assert_eq!(suggestions(cx).join(","), "directory,dir-link");
}

#[gpui_kit::test]
fn remote_directory_symlinks_survive_filter_while_files_and_other_kinds_do_not(
    cx: &mut TestAppContext,
) {
    let (handle, input, fs) = fixture(cx);
    start("/links/", handle, &input, cx);
    let mut directory = entry("directory-link", true);
    directory.is_symlink = true;
    let mut file = entry("file-link", false);
    file.is_symlink = true;
    let mut other = entry("special", false);
    other.kind = EntryKind::Other;
    take(&fs, "/links/")
        .send(Ok(vec![directory, file, other, entry("plain-file", false)]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/links/directory-link/");
    assert_eq!(suggestions(cx).len(), 1);
}

#[gpui_kit::test]
fn another_input_keeps_pointer_focus_and_draft_after_pending_navigation(cx: &mut TestAppContext) {
    let (handle, location, fs) = fixture(cx);
    let other = other_input(handle, &location, cx);
    start("/pending/", handle, &location, cx);
    let stale = take(&fs, "/pending/");
    cx.update_window(handle, |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(("input", other.entity_id()), cx);
        window.press("ctrl-a", cx);
        window.input("other draft", cx);
        assert!(other.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    cx.run_until_parked();
    let _ = stale.send(Ok(vec![entry("late completion", true)]));
    location.update(cx, |location, cx| {
        location.accept(Directory::Remote("/pending/".into()), None, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            other.read(cx).focus_handle(cx).is_focused(window),
            "Accepted navigation must not steal another field's focus"
        );
        assert!(window.try_find("explorer-path-editor").is_none());
        assert!(
            window
                .try_find(("path-completion-popup", location.entity_id()))
                .is_none()
        );
        window.input("!", cx);
        assert_eq!(other.read(cx).value().as_ref(), "other draft!");
    })
    .unwrap();
}

#[gpui_kit::test]
fn empty_folder_popup_enter_keeps_editing_and_later_plain_enter_closes(cx: &mut TestAppContext) {
    let (handle, location, fs) = fixture(cx);
    start("/empty/", handle, &location, cx);
    take(&fs, "/empty/")
        .send(Ok(vec![entry("file", false)]))
        .unwrap();
    cx.run_until_parked();
    assert!(visible(cx, "path-completion-popup"));
    assert!(suggestions(cx).is_empty());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find(("path-completion-popup", location.entity_id()))
                .visible()
        );
        window.press("up", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert!(!visible(cx, "path-completion-popup"));
    location.update(cx, |location, cx| {
        location.accept(Directory::Remote("/empty/".into()), None, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("explorer-path-editor").focused(), Some(true));
        assert!(
            window
                .try_find(("path-completion-popup", location.entity_id()))
                .is_none()
        );
        window.press("enter", cx);
    })
    .unwrap();
    assert!(!visible(cx, "path-completion-popup"));
    location.update(cx, |location, cx| {
        location.accept(Directory::Remote("/empty/".into()), None, cx)
    });
    cx.run_until_parked();
    assert!(visible(cx, "explorer-path-display"));
    assert!(
        !fs.has_pending_listings(),
        "Empty-list arrows must not create filesystem requests"
    );
}

#[gpui_kit::test]
fn arrows_without_popup_match_native_input_cursor_behavior(cx: &mut TestAppContext) {
    let (handle, location, fs) = fixture(cx);
    let other = other_input(handle, &location, cx);
    cx.update_window(handle, |_, window, cx| {
        for key in ["up", "down"] {
            window.click(("input", other.entity_id()), cx);
            window.press("home", cx);
            for _ in 0..3 {
                window.press("right", cx);
            }
            window.press(key, cx);
            window.input("!", cx);
            let expected = other.read(cx).value().to_string();
            other.update(cx, |other, cx| other.set_value("/work", window, cx));
            window.render_frame(cx);
            window.click("explorer-path-display", cx);
            window.press("home", cx);
            for _ in 0..3 {
                window.press("right", cx);
            }
            window.press(key, cx);
            window.input("!", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("explorer-path-editor").value(),
                Some(expected.as_str()),
                "{key} retains native Input caret behavior"
            );
            assert!(
                window
                    .try_find(("path-completion-popup", location.entity_id()))
                    .is_none()
            );
            window.press("escape", cx);
        }
    })
    .unwrap();
    assert!(!fs.has_pending_listings());
}

#[gpui_kit::test]
fn immediate_draft_edit_before_arrow_or_enter_uses_current_native_input(cx: &mut TestAppContext) {
    let (handle, location, fs) = fixture(cx);
    let mut results = Vec::new();
    for key in ["down", "enter"] {
        location.update(cx, |location, cx| {
            location.begin_navigation_with_mode(NavigationMode::CloseEditor, cx);
            location.accept(Directory::Remote("/work".into()), None, cx);
        });
        cx.run_until_parked();
        start("/etc/", handle, &location, cx);
        take(&fs, "/etc/")
            .send(Ok(vec![entry("a", true), entry("b", true)]))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(value(&location, cx), "/etc/a/");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Keep both native events in one update: Change notifications may
            // still be deferred when the following bound action executes.
            window.input("new", cx);
            window.press(key, cx);
        })
        .unwrap();
        results.push((
            key,
            value(&location, cx),
            visible(cx, "path-completion-popup"),
        ));
    }
    assert_eq!(
        results,
        [
            ("down", "/etc/a/new".into(), false),
            ("enter", "/etc/a/new".into(), false),
        ],
        "A typed draft invalidates the old popup before Arrow or Enter acts on it"
    );
}
