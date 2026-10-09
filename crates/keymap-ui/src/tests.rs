use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_keymap::Keymap;
use nocterm_settings::SettingsDocument;
use nocterm_ui::SettingsStore;

use crate::KeymapView;

const DEFAULTS: &str = r#"
[[section]]
context = "Workspace"
[section.bindings]
"ctrl-e" = "workspace::SwapSides"
"#;

fn setup(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<KeymapView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(SettingsDocument::default()),
            cx,
        );
        Keymap::init(DEFAULTS, None, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| KeymapView::new(window, cx))
        })
        .unwrap()
    })
}

fn bound(cx: &mut TestAppContext, keys: &str) -> Vec<String> {
    cx.update(|cx| {
        Keymap::global(cx)
            .bindings()
            .into_iter()
            .filter(|binding| binding.keystrokes == keys)
            .map(|binding| binding.action)
            .collect()
    })
}

#[gpui_kit::test]
fn pressing_keys_on_a_shortcut_rebinds_it_and_reset_restores_it(cx: &mut TestAppContext) {
    let (handle, view) = setup(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.search
                .update(cx, |search, cx| search.set_value("swap sides", window, cx))
        });
        window.render_frame(cx);
        assert!(
            window.try_find(("keymap-row", 1usize)).is_none(),
            "one match"
        );
        window.click(("keymap-keys", 0usize), cx);
        // The page takes the next keystroke, even one bound to a command.
        window.press("alt-s", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert!(bound(cx, "ctrl-e").is_empty());
    assert_eq!(bound(cx, "alt-s"), ["workspace::SwapSides"]);
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).recording.is_none());
        window.click(("keymap-reset", 0usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(bound(cx, "ctrl-e"), ["workspace::SwapSides"]);
    assert!(bound(cx, "alt-s").is_empty());
}

#[gpui_kit::test]
fn escape_cancels_recording_and_find_by_keys_fills_the_search(cx: &mut TestAppContext) {
    let (handle, view) = setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("keymap-search-keys", cx);
        window.press("ctrl-e", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).search.read(cx).value().as_ref(), "ctrl-e");
        assert!(window.try_find(("keymap-row", 0usize)).is_some());
        assert!(window.try_find(("keymap-row", 1usize)).is_none());
        window.click(("keymap-keys", 0usize), cx);
        window.press("escape", cx);
        assert!(view.read(cx).recording.is_none());
    })
    .unwrap();
    assert_eq!(bound(cx, "ctrl-e"), ["workspace::SwapSides"]);
}
