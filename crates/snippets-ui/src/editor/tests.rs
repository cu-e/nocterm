use super::*;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_ui::{Design, DesignTokens};
fn workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        crate::init(None, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        })
        .unwrap()
    })
}
#[gpui_kit::test]
fn multiline_enter_does_not_save_then_explicit_save_preserves_code(cx: &mut TestAppContext) {
    let (handle, workspace) = workspace(cx);
    let editor = cx
        .update_window(handle, |_, window, cx| {
            let editor = open(None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
            window.input("Disk usage", cx);
            let focus = editor.read(cx).code.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.render_frame(cx);
            window.input("echo first", cx);
            window.press("enter", cx);
            window.input("echo second", cx);
            editor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert!(Snippets::global(cx).read(cx).library.snippets.is_empty());
        assert!(editor.read(cx).code.read(cx).value().contains('\n'));
        editor.update(cx, |editor, cx| {
            editor.select_language(Language::Python, cx);
            editor.select_language(Language::Bash, cx);
        });
        window.render_frame(cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let model = Snippets::global(cx);
        let snippets = &model.read(cx).library.snippets;
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Disk usage");
        assert_eq!(snippets[0].content, "echo first\necho second");
        assert_eq!(snippets[0].language, Language::Bash);
    })
    .unwrap();
}
#[gpui_kit::test]
fn validation_and_cancel_keep_library_unchanged(cx: &mut TestAppContext) {
    let (handle, workspace) = workspace(cx);
    let editor = cx
        .update_window(handle, |_, window, cx| {
            let editor = open(None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
            window.click("snippet-save", cx);
            editor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert!(editor.read(cx).error.is_some());
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
#[test]
fn selected_languages_have_real_bundled_parsers() {
    use gpui_kit::component::{Rope, highlighter::SyntaxHighlighter};
    for (language, code) in [
        (Language::Bash, "echo \"hello\""),
        (Language::Json, "{\"ok\": true}"),
        (Language::Python, "print('hello')"),
        (Language::Yaml, "enabled: true\n"),
        (Language::Toml, "enabled = true\n"),
    ] {
        let mut highlighter = SyntaxHighlighter::new(language.id());
        assert_eq!(highlighter.language().as_ref(), language.id());
        assert!(highlighter.update(None, &Rope::from(code), None));
        assert!(highlighter.tree().is_some(), "{} must parse", language.id());
    }
}

#[gpui_kit::test]
fn save_under_stacked_dialog_keeps_both_dialogs_usable(cx: &mut TestAppContext) {
    let (handle, workspace) = workspace(cx);
    let editor = cx
        .update_window(handle, |_, window, cx| {
            let editor = open(None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
            editor.update(cx, |editor, cx| {
                editor
                    .name
                    .update(cx, |name, cx| name.set_value("Inspect", window, cx));
                editor
                    .code
                    .update(cx, |code, cx| code.set_value("echo first", window, cx));
                editor.save(window, cx);
            });
            // Application menus can stack an About dialog while the save is in flight.
            // Open it before the ready in-memory completion is delivered to the editor.
            window.open_dialog(cx, |dialog, _, _| {
                dialog.title("About").child(
                    gpui_kit::div()
                        .id("about-content")
                        .test_support()
                        .child("Nocterm"),
                )
            });
            assert!(
                !editor
                    .read(cx)
                    .dialog_focus
                    .as_ref()
                    .unwrap()
                    .contains_focused(window, cx),
                "About initially owns focus"
            );
            window.render_frame(cx);
            assert!(
                !editor
                    .read(cx)
                    .dialog_focus
                    .as_ref()
                    .unwrap()
                    .contains_focused(window, cx),
                "About owns focus after render"
            );
            editor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("about-content").is_some(),
            "Saving must preserve the newer dialog."
        );
        assert!(!editor.read(cx).dismissed.load(Ordering::Acquire));
        assert!(editor.read(cx).saved);
        assert!(!editor.read(cx).pending);
        assert_eq!(
            Snippets::global(cx).read(cx).library.snippets[0].content,
            "echo first"
        );
        window.close_dialog(cx);
        window.render_frame(cx);
        editor.update(cx, |editor, cx| {
            editor.code.update(cx, |code, cx| {
                code.focus(window, cx);
                code.set_selected_range(0..code.value().len(), cx);
            });
        });
        window.render_frame(cx);
        window.input("echo second", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!editor.read(cx).saved, "Editing clears the saved status.");
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("dialog").is_none(),
            "The resurfaced editor can save and close normally."
        );
        assert_eq!(
            Snippets::global(cx).read(cx).library.snippets[0].content,
            "echo second"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn attachment_navigation_preserves_draft_and_validation_returns_to_snippet(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = workspace(cx);
    let editor = cx
        .update_window(handle, |_, window, cx| {
            let editor = open(None, workspace.downgrade(), window, cx);
            editor.update(cx, |editor, cx| {
                editor.description.update(cx, |description, cx| {
                    description.set_value("First\nSecond", window, cx)
                });
                editor
                    .code
                    .update(cx, |code, cx| code.set_value("echo preserved", window, cx));
            });
            window.render_frame(cx);
            window.click("snippet-section-attachments", cx);
            editor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(matches!(
            editor.read(cx).section,
            EditorSection::Attachments
        ));
        editor.update(cx, |editor, cx| {
            editor.set_binding(true, "Work".into(), true, cx);
            assert!(
                editor.draft.profiles.is_empty(),
                "A group binding never expands into direct profile IDs."
            );
            editor.set_binding(false, "specific-server".into(), true, cx);
            editor.set_binding(true, "Work".into(), false, cx);
            assert_eq!(editor.draft.profiles, ["specific-server"]);
            editor
                .attachment_search
                .update(cx, |search, cx| search.set_value("unmatched", window, cx));
        });
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let draft = editor.read(cx);
        assert!(matches!(draft.section, EditorSection::Snippet));
        assert!(draft.name.read(cx).focus_handle(cx).is_focused(window));
        assert!(draft.error.is_some());
        assert_eq!(draft.description.read(cx).value().as_ref(), "First\nSecond");
        assert_eq!(draft.code.read(cx).value().as_ref(), "echo preserved");
        assert_eq!(draft.draft.profiles, ["specific-server"]);
        window.click("snippet-cancel", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimum_window_keeps_footer_visible_on_both_pages(cx: &mut TestAppContext) {
    let (handle, workspace) = workspace(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(400.)));
    cx.update_window(handle, |_, window, cx| {
        open(None, workspace.downgrade(), window, cx);
    })
    .unwrap();
    for section in ["snippet-section-snippet", "snippet-section-attachments"] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(section, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for button in ["snippet-save", "snippet-cancel"] {
                let bounds = window.find(button).bounds();
                assert!(
                    bounds.bottom() <= window.viewport_size().height,
                    "{section}: {button} must stay visible"
                );
            }
        })
        .unwrap();
    }
}
