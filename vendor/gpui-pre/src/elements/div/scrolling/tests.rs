// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;
use crate::{AnyWindowHandle, AppContext as _, Context, InputEvent, ScrollDelta, TestAppContext};

struct ScrollView {
    handle: ScrollHandle,
    content: Size<Pixels>,
    overflow: Point<Overflow>,
    restricted: bool,
    concurrent: bool,
    absolute: bool,
    padding: Pixels,
}

impl Render for ScrollView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let child = if self.absolute {
            div().absolute().size_full()
        } else {
            div()
                .w(self.content.width)
                .h(self.content.height)
                .flex_none()
        };
        let mut container = div()
            .id("scroll")
            .relative()
            .w(px(100.))
            .h(px(50.))
            .py(self.padding)
            .track_scroll(&self.handle)
            .child(child);
        container.style().overflow.x = Some(self.overflow.x);
        container.style().overflow.y = Some(self.overflow.y);
        container.style().restrict_scroll_to_axis = Some(self.restricted);
        container.style().allow_concurrent_scroll = Some(self.concurrent);
        div().size_full().child(container)
    }
}

fn setup(
    cx: &mut TestAppContext,
    content: Size<Pixels>,
    overflow: Point<Overflow>,
    restricted: bool,
    concurrent: bool,
    absolute: bool,
    padding: Pixels,
) -> (AnyWindowHandle, ScrollHandle) {
    let handle = ScrollHandle::new();
    let window = cx
        .add_window({
            let handle = handle.clone();
            move |_, _| ScrollView {
                handle,
                content,
                overflow,
                restricted,
                concurrent,
                absolute,
                padding,
            }
        })
        .into();
    draw(cx, window);
    (window, handle)
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
}

/// Returns event-caused invalidation immediately, with no intervening prepaint.
fn wheel(cx: &mut TestAppContext, window: AnyWindowHandle, x: f32, y: f32) -> usize {
    cx.update_window(window, |_, window, cx| {
        assert!(!window.invalidator.is_dirty());
        let before = window.invalidator.update_count();
        let result = window.dispatch_event(
            ScrollWheelEvent {
                position: point(px(10.), px(10.)),
                delta: ScrollDelta::Pixels(point(px(x), px(y))),
                modifiers: Default::default(),
                touch_phase: TouchPhase::Started,
            }
            .to_platform_input(),
            cx,
        );
        assert!(result.propagate, "Div scrolling must keep bubbling");
        let updates = window.invalidator.update_count() - before;
        assert_eq!(window.invalidator.is_dirty(), updates != 0);
        updates
    })
    .unwrap()
}

#[gpui::test]
fn dock_absolute_child_does_not_mutate_or_invalidate_before_prepaint(cx: &mut TestAppContext) {
    let (window, handle) = setup(
        cx,
        size(px(100.), px(50.)),
        point(Overflow::Hidden, Overflow::Scroll),
        false,
        false,
        true,
        px(0.),
    );
    assert_eq!(handle.max_offset(), point(px(0.), px(0.)));
    for y in [-200., 200., -1.] {
        assert_eq!(wheel(cx, window, 0., y), 0);
        assert_eq!(handle.offset(), point(px(0.), px(0.)));
    }
}

#[gpui::test]
fn vertical_boundaries_are_noops_and_interior_scroll_notifies(cx: &mut TestAppContext) {
    let (window, handle) = setup(
        cx,
        size(px(100.), px(150.)),
        point(Overflow::Hidden, Overflow::Scroll),
        true,
        false,
        false,
        px(0.),
    );
    assert_eq!(handle.max_offset().y, px(100.));
    assert_eq!(wheel(cx, window, 0., 10.), 0);
    assert_eq!(handle.offset().y, px(0.));
    assert_eq!(wheel(cx, window, 0., -30.), 1);
    assert_eq!(handle.offset().y, px(-30.));
    draw(cx, window);
    assert_eq!(wheel(cx, window, 0., -500.), 1);
    assert_eq!(handle.offset().y, px(-100.));
    draw(cx, window);
    assert_eq!(wheel(cx, window, 0., -10.), 0);
    assert_eq!(handle.offset().y, px(-100.));
    assert_eq!(wheel(cx, window, 0., 15.), 1);
    assert_eq!(handle.offset().y, px(-85.));
}

#[gpui::test]
fn horizontal_boundaries_and_axis_routing_are_preserved(cx: &mut TestAppContext) {
    for restricted in [true, false] {
        let (window, handle) = setup(
            cx,
            size(px(200.), px(50.)),
            point(Overflow::Scroll, Overflow::Hidden),
            restricted,
            false,
            false,
            px(0.),
        );
        assert_eq!(wheel(cx, window, 20., 0.), 0);
        assert_eq!(wheel(cx, window, 0., -25.), usize::from(!restricted));
        assert_eq!(
            handle.offset(),
            point(px(if restricted { 0. } else { -25. }), px(0.))
        );
        draw(cx, window);
        assert_eq!(wheel(cx, window, -500., 0.), 1);
        assert_eq!(handle.offset(), point(px(-100.), px(0.)));
        draw(cx, window);
        assert_eq!(wheel(cx, window, -10., 0.), 0);
        assert_eq!(handle.offset(), point(px(-100.), px(0.)));
    }
}

#[gpui::test]
fn vertical_axis_restriction_preserves_cross_axis_policy(cx: &mut TestAppContext) {
    for restricted in [true, false] {
        let (window, handle) = setup(
            cx,
            size(px(100.), px(150.)),
            point(Overflow::Hidden, Overflow::Scroll),
            restricted,
            false,
            false,
            px(0.),
        );
        assert_eq!(wheel(cx, window, -25., 0.), usize::from(!restricted));
        assert_eq!(
            handle.offset(),
            point(px(0.), px(if restricted { 0. } else { -25. }))
        );
    }
}

#[gpui::test]
fn concurrent_axis_policy_still_selects_dominant_or_both_axes(cx: &mut TestAppContext) {
    for concurrent in [false, true] {
        let (window, handle) = setup(
            cx,
            size(px(200.), px(150.)),
            point(Overflow::Scroll, Overflow::Scroll),
            false,
            concurrent,
            false,
            px(0.),
        );
        assert_eq!(wheel(cx, window, -20., -40.), 1);
        assert_eq!(
            handle.offset(),
            point(px(if concurrent { -20. } else { 0. }), px(-40.))
        );
    }
}

#[gpui::test]
fn fractional_padding_does_not_invalidate_a_fitting_container(cx: &mut TestAppContext) {
    let (window, handle) = setup(
        cx,
        size(px(100.), px(42.)),
        point(Overflow::Hidden, Overflow::Scroll),
        false,
        false,
        false,
        px(4.25),
    );
    assert_eq!(handle.max_offset().y, px(0.));
    assert_eq!(wheel(cx, window, 0., -10.), 0);
    assert_eq!(handle.offset().y, px(0.));
}

struct NestedScrollView {
    inner: ScrollHandle,
    outer: ScrollHandle,
}
impl Render for NestedScrollView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .id("outer")
                .w(px(100.))
                .h(px(100.))
                .overflow_y_scroll()
                .track_scroll(&self.outer)
                .child(
                    div().w(px(100.)).h(px(200.)).child(
                        div()
                            .id("inner")
                            .relative()
                            .w(px(100.))
                            .h(px(50.))
                            .overflow_y_scroll()
                            .track_scroll(&self.inner)
                            .child(div().absolute().size_full()),
                    ),
                ),
        )
    }
}

#[gpui::test]
fn fitting_inner_container_keeps_bubbling_to_scrollable_parent(cx: &mut TestAppContext) {
    let inner = ScrollHandle::new();
    let outer = ScrollHandle::new();
    let window = cx
        .add_window({
            let inner = inner.clone();
            let outer = outer.clone();
            move |_, _| NestedScrollView { inner, outer }
        })
        .into();
    draw(cx, window);
    assert_eq!(outer.max_offset().y, px(100.));
    assert_eq!(wheel(cx, window, 0., -20.), 1);
    assert_eq!(inner.offset().y, px(0.));
    assert_eq!(outer.offset().y, px(-20.));
    outer.set_offset(point(px(0.), px(-100.)));
    draw(cx, window);
    // At the parent boundary the same event still propagates, with no dirty frame.
    assert_eq!(wheel(cx, window, 0., -20.), 0);
    assert_eq!(inner.offset().y, px(0.));
    assert_eq!(outer.offset().y, px(-100.));
}
