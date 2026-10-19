use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_keymap::KeymapHandle;
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;

use crate::KeymapView;

const DEFAULTS: &str = r#"
[[section]]
context = "Workspace"
[section.bindings]
"ctrl-e" = "workspace::SwapSides"
"#;

fn setup(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<KeymapView>, KeymapHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            (SettingsConfig::in_memory(SettingsDocument::default())).install(cx),
            cx,
        );
        let keymap = KeymapHandle::install(DEFAULTS, None, cx);
        let (window, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| KeymapView::new(keymap.clone(), window, cx))
        })
        .unwrap();
        (window, view, keymap)
    })
}

fn bound(keymap: &KeymapHandle, cx: &mut TestAppContext, keys: &str) -> Vec<String> {
    cx.update(|cx| {
        keymap
            .bindings(cx)
            .into_iter()
            .filter(|binding| binding.keystrokes == keys)
            .map(|binding| binding.action)
            .collect()
    })
}

#[gpui_kit::test]
fn pressing_keys_on_a_shortcut_rebinds_it_and_reset_restores_it(cx: &mut TestAppContext) {
    let (handle, _view, keymap) = setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("keymap-search", cx);
        window.input("swap sides", cx);
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
    assert!(bound(&keymap, cx, "ctrl-e").is_empty());
    assert_eq!(bound(&keymap, cx, "alt-s"), ["workspace::SwapSides"]);
    cx.update_window(handle, |_, window, cx| {
        window.click(("keymap-reset", 0usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(bound(&keymap, cx, "ctrl-e"), ["workspace::SwapSides"]);
    assert!(bound(&keymap, cx, "alt-s").is_empty());
}

#[gpui_kit::test]
fn escape_cancels_recording_and_find_by_keys_fills_the_search(cx: &mut TestAppContext) {
    let (handle, _view, keymap) = setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("keymap-search-keys", cx);
        window.press("ctrl-e", cx);
        window.render_frame(cx);
        assert!(window.try_find(("keymap-row", 0usize)).is_some());
        assert!(window.try_find(("keymap-row", 1usize)).is_none());
        window.click(("keymap-keys", 0usize), cx);
        window.press("escape", cx);
        window.press("alt-s", cx);
    })
    .unwrap();
    assert_eq!(bound(&keymap, cx, "ctrl-e"), ["workspace::SwapSides"]);
    assert!(bound(&keymap, cx, "alt-s").is_empty());
}

fn shared_page(
    keymap: &KeymapHandle,
    cx: &mut TestAppContext,
) -> (gpui_kit::AnyWindowHandle, Entity<KeymapView>) {
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| KeymapView::new(keymap.clone(), window, cx))
        })
        .unwrap()
    })
}
fn search_swap(handle: gpui_kit::AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("keymap-search", cx);
        window.input("swap sides", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}
fn close_page(
    handle: gpui_kit::AnyWindowHandle,
    view: Entity<KeymapView>,
    cx: &mut TestAppContext,
) {
    let weak = view.downgrade();
    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    cx.run_until_parked();
    assert!(
        weak.upgrade().is_none(),
        "shared observer/interceptor must not retain closed page"
    );
}
#[gpui_kit::test]
fn shared_pages_refresh_survive_window_closure_and_release_recording(cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    let (first, first_view, keymap) = setup(cx);
    let (second, second_view) = shared_page(&keymap, cx);
    search_swap(first, cx);
    search_swap(second, cx);
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    let _observed = cx.update(|cx| cx.observe(&second_view, move |_, _| seen.set(seen.get() + 1)));
    cx.update_window(first, |_, window, cx| {
        window.click(("keymap-keys", 0usize), cx);
        window.press("alt-s", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        changes.get() > 0,
        "other page receives owner notification without local input"
    );
    cx.update_window(second, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("keymap-reset", 0usize)).is_some());
    })
    .unwrap();
    close_page(first, first_view, cx);
    cx.update_window(second, |_, window, cx| {
        window.click(("keymap-reset", 0usize), cx);
        window.render_frame(cx);
        assert!(window.try_find(("keymap-reset", 0usize)).is_none());
        window.click(("keymap-keys", 0usize), cx);
        window.press("alt-t", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (third, third_view) = shared_page(&keymap, cx);
    search_swap(third, cx);
    cx.update_window(third, |_, window, cx| {
        assert!(window.try_find(("keymap-reset", 0usize)).is_some());
        window.click(("keymap-keys", 0usize), cx);
    })
    .unwrap();
    close_page(second, second_view, cx);
    close_page(third, third_view, cx);
    assert_eq!(bound(&keymap, cx, "alt-t"), ["workspace::SwapSides"]);
}
