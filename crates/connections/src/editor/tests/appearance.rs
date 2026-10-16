//! The actual connection dialog and icon fade use the supplied terminal surface.
use super::*;

#[gpui_kit::test]
fn dialog_and_icon_fade_paint_without_an_ambient_settings_alias(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let custom = nocterm_ui::DesignTokens::with_overrides(
        "[light.terminal]\nbackground = '#123456'\n[dark.terminal]\nbackground = '#123456'\n",
    )
    .unwrap();
    let expected = nocterm_ui::hsla("#123456".parse::<nocterm_ui::Color>().unwrap());
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.set_global(nocterm_ui::Design::new(custom));
        cx.remove_global::<nocterm_ui::SettingsStore>();
    });
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1800.)));
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed, None, workspace.downgrade(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let dialog = window.within("dialog").find(0usize).bounds().scale(window.scale_factor());
        let quads = window.painted_quads();
        assert!(
            quads
                .iter()
                .any(|q| q.bounds == dialog && q.background.as_solid() == Some(expected)),
            "actual dialog paints the supplied background: bounds={dialog:?}, expected={expected:?}, matching={:?}, overlap={:?}",
            quads.iter().filter(|q| q.background.as_solid()==Some(expected)).map(|q| q.bounds).collect::<Vec<_>>(),
            quads.iter().filter(|q| q.bounds.intersects(&dialog)).take(10).map(|q| (q.bounds,q.background)).collect::<Vec<_>>()
        );
        window.scroll(
            "editor-section-content",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-800.))),
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("editor-icon-auto").visible(), "icon grid must actually be visible");
        let quads = window.painted_quads();
        let fade = gpui_kit::linear_gradient(
            180.,
            gpui_kit::linear_color_stop(expected.opacity(0.), 0.),
            gpui_kit::linear_color_stop(expected, 1.),
        );
        assert!(
            quads.iter().any(|q| q.background == fade),
            "collapsed native icon grid paints the original terminal fade"
        );
    })
    .unwrap();
}
