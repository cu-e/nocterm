//! Directory-only completion and native keyboard/navigation lifecycle.
use super::{tests::*, *};
use gpui_kit::{TestAppContext, px, test::TestWindowExt as _};

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
    let reply = fs.take("/etc/");
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
        assert_eq!(input.read(cx).cycle.as_ref().unwrap().selected, Some(0));
        window.press("up", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/etc/b/");
    start("/single/", handle, &input, cx);
    fs.take("/single/")
        .send(Ok(vec![entry("only", true)]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("up", cx);
        window.press("down", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/single/only/");
    assert!(input.read_with(cx, |input, _| input.pending.is_none()));
}

#[gpui_kit::test]
fn popup_enter_keeps_absolute_unicode_value_caret_then_plain_enter_closes(cx: &mut TestAppContext) {
    let (handle, input, fs) = fixture(cx);
    start("~/", handle, &input, cx);
    fs.take("/home/test/")
        .send(Ok(vec![entry("資料 folder", true)]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    assert_eq!(
        input.read_with(cx, |input, _| input.navigation_mode),
        NavigationMode::KeepEditing
    );
    input.update(cx, |input, cx| {
        input.accept(Directory::Remote("/home/test/資料 folder".into()), None, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let editor = input.read(cx);
        assert!(editor.editing && !editor.popup_visible());
        assert!(editor.input.read(cx).focus_handle(cx).is_focused(window));
        let end = "/home/test/資料 folder".len();
        assert_eq!(editor.input.read(cx).selected_range(), end..end);
        window.input("/next", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(value(&input, cx), "/home/test/資料 folder/next");
    assert_eq!(
        input.read_with(cx, |input, _| input.navigation_mode),
        NavigationMode::CloseEditor
    );
    input.update(cx, |input, cx| {
        input.accept(
            Directory::Remote("/home/test/資料 folder/next".into()),
            None,
            cx,
        )
    });
    cx.run_until_parked();
    assert!(!input.read_with(cx, |input, _| input.editing));
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
        let reply = fs.take("/pending/");
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
            input.read_with(cx, |input, _| !input.editing && !input.popup_visible()),
            "escape={escape}"
        );
        cx.update_window(handle, |_, window, cx| {
            if !escape {
                assert!(!input.read(cx).focus.is_focused(window));
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
        // Shape the displayed text with the inherited display font at its shared
        // pixel size, then compare the native editor's actual glyph geometry.
        let style = window.text_style();
        let displayed = "/work";
        let shaped = window.text_system().shape_line(
            displayed.into(),
            px(12.),
            &[gpui_kit::TextRun {
                len: displayed.len(),
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        window.click("explorer-path-display", cx);
        window.render_frame(cx);
        let editor = input.read(cx).input.clone();
        let frame = window.find(("input", editor.entity_id())).bounds();
        let text = editor.read(cx).text_bounds().unwrap();
        assert_eq!(display.size, frame.size);
        assert_eq!(editor.read(cx).line_height(), Some(px(18.)));
        let glyphs = editor
            .read(cx)
            .range_to_bounds(&(0..displayed.len()))
            .unwrap();
        assert!(
            (glyphs.size.width - shaped.width).abs() <= px(0.01),
            "Native editor must retain the displayed font and 12px glyph size"
        );
        assert!((text.left() - display.left() - px(9.)).abs() <= px(1.));
        assert!((text.center().y - display.center().y).abs() <= px(1.));
        input.update(cx, |input, cx| {
            input.begin_navigation_with_mode(NavigationMode::KeepEditing, cx)
        });
        window.render_frame(cx);
        assert_eq!(window.find(("input", editor.entity_id())).bounds(), frame);
        assert!(editor.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[cfg(unix)]
#[gpui_kit::test]
fn local_only_directory_symlinks_are_suggested(cx: &mut TestAppContext) {
    use std::os::unix::fs::symlink;
    let (handle, input, _) = fixture(cx);
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("directory")).unwrap();
    std::fs::write(root.path().join("file"), b"text").unwrap();
    symlink("directory", root.path().join("dir-link")).unwrap();
    symlink("file", root.path().join("file-link")).unwrap();
    symlink("missing", root.path().join("broken-link")).unwrap();
    input.update(cx, |input, cx| {
        input.accept(Directory::Local(root.path().into()), None, cx)
    });
    start("", handle, &input, cx);
    cx.run_until_parked();
    assert_eq!(
        input.read_with(cx, |input, _| input
            .cycle
            .as_ref()
            .unwrap()
            .candidates
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>()
            .join(",")),
        "directory,dir-link"
    );
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
    fs.take("/links/")
        .send(Ok(vec![directory, file, other, entry("plain-file", false)]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(&input, cx), "/links/directory-link/");
    assert_eq!(
        input.read_with(cx, |input, _| input
            .cycle
            .as_ref()
            .unwrap()
            .candidates
            .len()),
        1
    );
}

#[gpui_kit::test]
fn another_input_keeps_pointer_focus_and_draft_after_pending_navigation(cx: &mut TestAppContext) {
    let (handle, location, fs) = fixture(cx);
    let other = other_input(handle, &location, cx);
    start("/pending/", handle, &location, cx);
    let stale = fs.take("/pending/");
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
        assert!(!location.read(cx).editing && !location.read(cx).popup_visible());
        window.input("!", cx);
        assert_eq!(other.read(cx).value().as_ref(), "other draft!");
    })
    .unwrap();
}

#[gpui_kit::test]
fn empty_folder_popup_enter_keeps_editing_and_later_plain_enter_closes(cx: &mut TestAppContext) {
    let (handle, location, fs) = fixture(cx);
    start("/empty/", handle, &location, cx);
    fs.take("/empty/")
        .send(Ok(vec![entry("file", false)]))
        .unwrap();
    cx.run_until_parked();
    assert!(
        location.read_with(cx, |location, _| location.popup_visible()
            && location.cycle.as_ref().unwrap().candidates.is_empty())
    );
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
    assert_eq!(
        location.read_with(cx, |location, _| location.navigation_mode),
        NavigationMode::KeepEditing
    );
    location.update(cx, |location, cx| {
        location.accept(Directory::Remote("/empty/".into()), None, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(location.read(cx).editing && !location.read(cx).popup_visible());
        assert!(
            location
                .read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(
        location.read_with(cx, |location, _| location.navigation_mode),
        NavigationMode::CloseEditor
    );
    location.update(cx, |location, cx| {
        location.accept(Directory::Remote("/empty/".into()), None, cx)
    });
    cx.run_until_parked();
    assert!(!location.read_with(cx, |location, _| location.editing));
    assert!(
        !fs.has_requests(),
        "Empty-list arrows must not create filesystem requests"
    );
}

#[gpui_kit::test]
fn arrows_without_popup_match_native_input_cursor_behavior(cx: &mut TestAppContext) {
    let (handle, location, fs) = fixture(cx);
    let other = other_input(handle, &location, cx);
    cx.update_window(handle, |_, window, cx| {
        location.update(cx, |location, cx| location.edit(window, cx));
        window.render_frame(cx);
        let editor = location.read(cx).input.clone();
        for key in ["up", "down"] {
            other.update(cx, |other, cx| {
                other.focus(window, cx);
                other.set_selected_range(3..3, cx);
            });
            window.render_frame(cx);
            window.press(key, cx);
            let expected = other.read(cx).selected_range();
            location.update(cx, |location, cx| location.edit(window, cx));
            editor.update(cx, |editor, cx| editor.set_selected_range(3..3, cx));
            window.render_frame(cx);
            assert!(!location.read(cx).popup_visible());
            window.press(key, cx);
            assert_eq!(
                editor.read(cx).selected_range(),
                expected,
                "{key} should retain native Input behavior without completion"
            );
            assert_eq!(editor.read(cx).value().as_ref(), "/work");
        }
    })
    .unwrap();
    assert!(!fs.has_requests());
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
        fs.take("/etc/")
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
            location.read_with(cx, |location, _| location.navigation_mode),
        ));
    }
    assert_eq!(
        results,
        [
            ("down", "/etc/a/new".into(), NavigationMode::CloseEditor),
            ("enter", "/etc/a/new".into(), NavigationMode::CloseEditor),
        ],
        "A typed draft invalidates the old popup before Arrow or Enter acts on it"
    );
}
