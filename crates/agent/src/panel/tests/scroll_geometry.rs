//! Transcript wheel geometry with cold and changing row measurements.
use super::*;
use gpui_kit::{
    MouseMoveEvent, ScrollDelta, ScrollWheelEvent, StyleRefinement, TestSupportExt as _,
    VisualTestContext,
    component::message_scroller::{MessageScroller, MessageScrollerState},
    div, point, px, size,
};
use std::cell::RefCell;

struct Rows {
    state: Entity<MessageScrollerState>,
    heights: Rc<RefCell<Vec<f32>>>,
    rendered: Rc<std::cell::Cell<usize>>,
}
impl Render for Rows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let heights = self.heights.clone();
        let rendered = self.rendered.clone();
        MessageScroller::new("geometry", self.state.clone(), move |index, _, _| {
            rendered.set(rendered.get() + 1);
            div().w_full().h(px(heights.borrow()[index]))
        })
        .with_row_style(StyleRefinement::default().p_0())
        .with_list_style(StyleRefinement::default().p_0())
        .jump_button(false)
        .scrollbar(false)
    }
}
fn paint<V: Render>(cx: &mut VisualTestContext, view: &Entity<V>, width: f32) {
    cx.draw(point(px(0.), px(0.)), size(px(width), px(100.)), |_, _| {
        view.clone().into_any_element()
    });
}
fn wheel(cx: &mut VisualTestContext, delta: f32) {
    let position = point(px(50.), px(50.));
    cx.simulate_event(MouseMoveEvent {
        position,
        ..Default::default()
    });
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
        ..Default::default()
    });
}
fn position(state: &Entity<MessageScrollerState>, heights: &[f32], cx: &VisualTestContext) -> f32 {
    let top = state.read_with(cx, |state, _| state.logical_scroll_top());
    heights[..top.item_ix].iter().sum::<f32>() + f32::from(top.offset_in_item)
}

#[gpui_kit::test]
fn a_cold_history_preserves_upward_wheel_distance_across_unseen_rows(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 512]));
    let state = cx.new(|cx| MessageScrollerState::new(512, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    let mut expected = 512. * 20. - 100.;
    assert_eq!(position(&state, &heights.borrow(), cx), expected);
    for distance in [40., 240., 800., 1600.] {
        wheel(cx, distance);
        paint(cx, &view, 120.);
        expected -= distance;
        assert_eq!(
            position(&state, &heights.borrow(), cx),
            expected,
            "a {distance}px upward wheel event must traverse that distance in cold history"
        );
    }
}

#[gpui_kit::test]
fn upward_wheel_enters_the_bottom_of_a_row_that_grew_offscreen(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 512]));
    let state = cx.new(|cx| MessageScrollerState::new(512, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    // A previously visited history has cached heights for the older rows.
    for index in (0..512).step_by(20) {
        state.update(cx, |state, cx| {
            state.scroll_to_item(index, cx);
        });
        paint(cx, &view, 120.);
    }
    state.update(cx, |state, cx| state.scroll_to_end(cx));
    paint(cx, &view, 120.);
    heights.borrow_mut()[470] = 3000.;
    state.update(cx, |state, cx| {
        state.remeasure_items(470..471, cx);
    });
    paint(cx, &view, 120.);
    let before = position(&state, &heights.borrow(), cx);
    wheel(cx, 800.);
    paint(cx, &view, 120.);
    assert_eq!(
        position(&state, &heights.borrow(), cx),
        before - 800.,
        "wheel must enter the grown row from its bottom, not jump past its new content"
    );
}

#[gpui_kit::test]
fn growing_a_row_below_the_viewport_preserves_downward_wheel_distance(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 512]));
    let state = cx.new(|cx| MessageScrollerState::new(512, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    for index in (0..512).step_by(20) {
        state.update(cx, |state, cx| {
            state.scroll_to_item(index, cx);
        });
        paint(cx, &view, 120.);
    }
    state.update(cx, |state, cx| {
        state.scroll_to_item(0, cx);
    });
    paint(cx, &view, 120.);
    heights.borrow_mut()[40] = 3000.;
    state.update(cx, |state, cx| {
        state.remeasure_items(40..41, cx);
    });
    paint(cx, &view, 120.);
    wheel(cx, -1000.);
    paint(cx, &view, 120.);
    assert_eq!(position(&state, &heights.borrow(), cx), 1000.);
}

#[gpui_kit::test]
fn width_invalidation_does_not_make_upward_wheel_skip_newly_wrapped_text(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 512]));
    let state = cx.new(|cx| MessageScrollerState::new(512, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    for index in (0..512).step_by(20) {
        state.update(cx, |state, cx| {
            state.scroll_to_item(index, cx);
        });
        paint(cx, &view, 240.);
    }
    state.update(cx, |state, cx| state.scroll_to_end(cx));
    paint(cx, &view, 240.);
    heights.borrow_mut()[470] = 3000.;
    paint(cx, &view, 120.);
    let before = position(&state, &heights.borrow(), cx);
    wheel(cx, 800.);
    paint(cx, &view, 120.);
    assert_eq!(position(&state, &heights.borrow(), cx), before - 800.);
}

#[gpui_kit::test]
fn opposite_wheel_events_before_paint_coalesce_without_losing_distance(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 512]));
    let state = cx.new(|cx| MessageScrollerState::new(512, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    let before = position(&state, &heights.borrow(), cx);
    wheel(cx, 800.);
    wheel(cx, -400.);
    paint(cx, &view, 120.);
    assert_eq!(position(&state, &heights.borrow(), cx), before - 400.);
}

#[gpui_kit::test]
fn explicit_navigation_cancels_wheel_continuation(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 1024]));
    let state = cx.new(|cx| MessageScrollerState::new(1024, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    wheel(cx, 100_000.);
    paint(cx, &view, 120.);
    assert!(
        position(&state, &heights.borrow(), cx) > 0.,
        "a large gesture must spread missing measurements across frames"
    );
    state.update(cx, |state, cx| {
        state.scroll_to_item(400, cx);
    });
    for _ in 0..3 {
        paint(cx, &view, 120.);
    }
    assert_eq!(position(&state, &heights.borrow(), cx), 8000.);
}

struct NativeRows {
    state: gpui_kit::ListState,
    heights: Rc<RefCell<Vec<f32>>>,
}
impl Render for NativeRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let heights = self.heights.clone();
        gpui_kit::list(self.state.clone(), move |index, _, _| {
            div()
                .id(("native-row", index))
                .test_support()
                .w_full()
                .h(px(heights.borrow()[index]))
                .into_any_element()
        })
        .size_full()
    }
}

#[gpui_kit::test]
fn a_bottom_aligned_native_list_stays_pinned_after_wheel_at_its_edge(cx: &mut TestAppContext) {
    let heights = Rc::new(RefCell::new(vec![20.; 10]));
    let state = gpui_kit::ListState::new(10, gpui_kit::ListAlignment::Bottom, px(400.));
    let view = cx.new(|_| NativeRows {
        state: state.clone(),
        heights: heights.clone(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    wheel(cx, 20.);
    paint(cx, &view, 120.);
    assert_eq!(state.bounds_for_item(9).unwrap().bottom(), px(120.));
    wheel(cx, -20.);
    paint(cx, &view, 120.);
    wheel(cx, -20.);
    heights.borrow_mut().push(20.);
    state.splice(10..10, 1);
    paint(cx, &view, 120.);
    assert_eq!(
        cx.update(|window, _| window.find(("native-row", 10usize)).bounds().bottom()),
        px(100.)
    );
}

#[gpui_kit::test]
fn a_fitting_chat_keeps_following_when_upward_wheel_cannot_move_it(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 3]));
    let state = cx.new(|cx| MessageScrollerState::new(3, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    state.update(cx, |state, cx| state.scroll_to_end(cx));
    wheel(cx, 20.);
    assert!(state.read_with(cx, |state, _| state.is_following_tail()));
    heights.borrow_mut().push(300.);
    state.update(cx, |state, cx| {
        state.append(1, cx);
    });
    paint(cx, &view, 120.);
    assert_eq!(position(&state, &heights.borrow(), cx), 260.);
}

#[gpui_kit::test]
fn a_deferred_native_wheel_landing_at_the_end_keeps_bottom_pinning(cx: &mut TestAppContext) {
    let heights = Rc::new(RefCell::new(vec![20.; 10]));
    let state = gpui_kit::ListState::new(10, gpui_kit::ListAlignment::Bottom, px(400.));
    let view = cx.new(|_| NativeRows {
        state: state.clone(),
        heights: heights.clone(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    state.scroll_to(gpui_kit::ListOffset {
        item_ix: 0,
        offset_in_item: px(0.),
    });
    paint(cx, &view, 120.);
    state.remeasure_items(4..5);
    wheel(cx, -100.);
    paint(cx, &view, 120.);
    heights.borrow_mut().push(20.);
    state.splice(10..10, 1);
    paint(cx, &view, 120.);
    assert_eq!(
        cx.update(|window, _| window.find(("native-row", 10usize)).bounds().bottom()),
        px(100.)
    );
}

#[gpui_kit::test]
fn a_large_wheel_schedules_bounded_measurement_batches_and_then_goes_idle(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 1024]));
    let rendered = Rc::new(std::cell::Cell::new(0usize));
    let state = cx.new(|cx| MessageScrollerState::new(1024, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: rendered.clone(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    wheel(cx, 100_000.);
    for _ in 0..12 {
        rendered.set(0);
        paint(cx, &view, 120.);
        assert!(
            rendered.get() <= 160,
            "normal positive-height rows must not trigger eager history rendering: {}",
            rendered.get()
        );
        assert!(
            cx.update(|window, cx| window.simulate_next_frame(cx)) > 0,
            "missing-height continuation and final controls refresh need a real frame request"
        );
        if position(&state, &heights.borrow(), cx) == 0. {
            paint(cx, &view, 120.);
            assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 0);
            return;
        }
    }
    panic!("queued movement did not finish");
}

#[gpui_kit::test]
fn reversing_at_the_tail_cancels_an_unmeasured_upward_step(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let heights = Rc::new(RefCell::new(vec![20.; 512]));
    let state = cx.new(|cx| MessageScrollerState::new(512, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    let before = position(&state, &heights.borrow(), cx);
    state.update(cx, |state, cx| {
        state.remeasure_items(506..507, cx);
    });
    wheel(cx, 20.);
    wheel(cx, -40.);
    paint(cx, &view, 120.);
    assert_eq!(position(&state, &heights.borrow(), cx), before);
}

#[gpui_kit::test]
fn zero_height_rows_do_not_reengage_tail_follow_before_a_wheel_finishes(cx: &mut TestAppContext) {
    let _f = fixture(cx);
    let mut initial = vec![20.; 402];
    initial[0] = 1000.;
    initial[401] = 100.;
    let heights = Rc::new(RefCell::new(initial));
    let state = cx.new(|cx| MessageScrollerState::new(402, cx));
    let view = cx.new(|_| Rows {
        state: state.clone(),
        heights: heights.clone(),
        rendered: Default::default(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    heights.borrow_mut()[1..401].fill(0.);
    state.update(cx, |state, cx| {
        state.remeasure_items(1..401, cx);
    });
    wheel(cx, 20.);
    for _ in 0..4 {
        paint(cx, &view, 120.);
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
    }
    assert_eq!(position(&state, &heights.borrow(), cx), 980.);
    assert!(!state.read_with(cx, |state, _| state.is_following_tail()));
}

#[gpui_kit::test]
fn explicit_list_replacement_and_thumb_drag_cancel_deferred_wheel(cx: &mut TestAppContext) {
    for action in ["reset", "replace anchor", "drag thumb"] {
        let heights = Rc::new(RefCell::new(vec![20.; 1024]));
        let state = gpui_kit::ListState::new(1024, gpui_kit::ListAlignment::Top, px(400.));
        let view = cx.new(|_| NativeRows {
            state: state.clone(),
            heights: heights.clone(),
        });
        let cx = cx.add_empty_window();
        state.scroll_to_end();
        paint(cx, &view, 120.);
        wheel(cx, 100_000.);
        let anchor = state.logical_scroll_top();
        let expected = match action {
            "reset" => {
                state.reset(1024);
                0
            }
            "replace anchor" => {
                state.splice(anchor.item_ix..anchor.item_ix + 1, 1);
                anchor.item_ix
            }
            "drag thumb" => {
                state.scrollbar_drag_started();
                anchor.item_ix
            }
            _ => unreachable!(),
        };
        for _ in 0..3 {
            paint(cx, &view, 120.);
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
        }
        assert_eq!(
            state.logical_scroll_top().item_ix,
            expected,
            "{action} must cancel old wheel input"
        );
        assert_eq!(state.logical_scroll_top().offset_in_item, px(0.));
    }
}

#[gpui_kit::test]
fn upward_wheel_immediately_after_jump_to_latest_starts_at_the_viewport_tail(
    cx: &mut TestAppContext,
) {
    let _f = fixture(cx);
    for append_before_paint in [false, true] {
        let heights = Rc::new(RefCell::new(vec![20.; 512]));
        let state = cx.new(|cx| MessageScrollerState::new(512, cx));
        let view = cx.new(|_| Rows {
            state: state.clone(),
            heights: heights.clone(),
            rendered: Default::default(),
        });
        let cx = cx.add_empty_window();
        paint(cx, &view, 120.);
        state.update(cx, |state, cx| {
            state.scroll_to_item(0, cx);
        });
        paint(cx, &view, 120.);
        state.update(cx, |state, cx| state.scroll_to_end(cx));
        if append_before_paint {
            heights.borrow_mut().push(300.);
            state.update(cx, |state, cx| state.append(1, cx));
        }
        // Input arrives after the action but before the end request is painted.
        wheel(cx, 20.);
        paint(cx, &view, 120.);
        assert_eq!(
            position(&state, &heights.borrow(), cx),
            heights.borrow().iter().sum::<f32>() - 100. - 20.
        );
        assert!(!state.read_with(cx, |state, _| state.is_following_tail()));
    }
}

#[gpui_kit::test]
fn an_inert_wheel_preserves_a_native_end_request_through_append(cx: &mut TestAppContext) {
    let heights = Rc::new(RefCell::new(vec![20.; 3]));
    let state = gpui_kit::ListState::new(3, gpui_kit::ListAlignment::Top, px(400.));
    let view = cx.new(|_| NativeRows {
        state: state.clone(),
        heights: heights.clone(),
    });
    let cx = cx.add_empty_window();
    paint(cx, &view, 120.);
    state.scroll_to_end();
    wheel(cx, 20.);
    heights.borrow_mut().push(300.);
    state.splice(3..3, 1);
    paint(cx, &view, 120.);
    assert_eq!(state.bounds_for_item(3).unwrap().bottom(), px(100.));
}
