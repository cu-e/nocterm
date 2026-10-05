use super::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Context, Entity, StyleRefinement, TestAppContext, WindowOptions, div, prelude::*};
use std::cell::Cell;

struct CountedNotices(Rc<Cell<usize>>);
impl gpui_kit::Render for CountedNotices {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.set(self.0.get() + 1);
        div().size_full().child(NoticeBar::new())
    }
}
struct CachedHost(Entity<CountedNotices>);
impl gpui_kit::Render for CachedHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0
            .clone()
            .cached(StyleRefinement::default().size_full())
    }
}

#[gpui_kit::test]
fn absent_notice_operations_do_not_force_cached_views_to_redraw(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let handle = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init(
            crate::DesignTokens::builtin(),
            crate::SettingsStore::in_memory(Default::default()),
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            let child = cx.new(|_| CountedNotices(renders.clone()));
            cx.new(|_| CachedHost(child))
        })
        .unwrap()
        .0
    });
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        let before = renders.get();
        remove(window, cx, "absent");
        clear(window, cx);
        set_expanded(window, cx, false);
        window.draw(cx).clear(cx);
        assert_eq!(
            renders.get(),
            before,
            "no-op operations must preserve cached views"
        );
        warning_action(
            window,
            cx,
            "present",
            "Waiting",
            "Short",
            "Review",
            |_, _| {},
        );
        window.draw(cx).clear(cx);
        assert!(renders.get() > before);
        let before = renders.get();
        remove(window, cx, "absent");
        set_expanded(window, cx, false);
        window.draw(cx).clear(cx);
        assert_eq!(
            renders.get(),
            before,
            "removing an absent key must not refresh the window"
        );
        set_expanded(window, cx, true);
        remove(window, cx, "present");
        assert!(
            !notices(window, cx).expanded,
            "removing the last notice also closes the list"
        );
        window.draw(cx).clear(cx);
        assert!(
            renders.get() > before,
            "a real removal must invalidate the footer"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn reduced_motion_marquee_rests_at_start_and_survives_redraw(cx: &mut TestAppContext) {
    let handle = super::tests::window(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    cx.update_window(handle, |_, window, cx| {
        warning_action(window, cx, "long", "Waiting", "A long operational notice that should scroll past a few times, then settle at its beginning while the operation keeps waiting for user action.", "Review", |_, _| {});
        window.render_frame(cx);
        assert!(window.find("notice-marquee").visible());
    }).unwrap();

    let start = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let viewport = window.find("notice-text").bounds();
            let row = window.find("notice-marquee").bounds();
            assert_eq!(
                row.origin.x, viewport.origin.x,
                "reduced motion text rests at its start"
            );
            row.origin.x
        })
        .unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("notice-marquee").bounds().origin.x,
            start,
            "ordinary redraws preserve the resting position"
        );
    })
    .unwrap();
}
