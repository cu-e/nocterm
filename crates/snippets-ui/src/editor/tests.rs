use super::*;
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
