// SPDX-License-Identifier: Apache-2.0
// Nocterm regression: native wheel extent retains estimates through layout invalidation.
use gpui::{
    AppContext, Context, Entity, IntoElement, ListAlignment, ListState, MouseMoveEvent, Render,
    ScrollDelta, ScrollWheelEvent, Styled, TestAppContext, VisualTestContext, Window, div, list,
    point, px, size,
};
use std::{cell::RefCell, rc::Rc};

struct Rows {
    state: ListState,
    rendered: Rc<RefCell<Vec<usize>>>,
}
impl Render for Rows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rendered = self.rendered.clone();
        list(self.state.clone(), move |index, _, _| {
            rendered.borrow_mut().push(index);
            div().h(px(20.)).w_full().into_any_element()
        })
        .size_full()
    }
}
fn paint(
    cx: &mut VisualTestContext,
    view: &Entity<Rows>,
    rendered: &Rc<RefCell<Vec<usize>>>,
    width: f32,
) {
    rendered.borrow_mut().clear();
    cx.draw(point(px(0.), px(0.)), size(px(width), px(100.)), |_, _| {
        view.clone().into_any_element()
    });
}
fn wheel(cx: &mut VisualTestContext, vertical: f32) {
    let position = point(px(5.), px(5.));
    cx.simulate_event(MouseMoveEvent {
        position,
        pressed_button: None,
        modifiers: Default::default(),
    });
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(vertical))),
        ..Default::default()
    });
}

#[gpui::test]
fn native_wheel_reaches_unseen_rows_after_first_paint_and_width_change(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let rendered = Rc::new(RefCell::new(Vec::new()));
    let state = ListState::new(256, ListAlignment::Top, px(0.)).with_uniform_item_height(px(20.));
    let observed = rendered.clone();
    let view = cx.update(|_, cx| {
        cx.new(|_| Rows {
            state,
            rendered: observed,
        })
    });
    for width in [120., 240.] {
        paint(cx, &view, &rendered, width);
        assert!(rendered.borrow().contains(&0));
        assert!(!rendered.borrow().contains(&255));
        assert!(
            rendered.borrow().len() < 16,
            "only viewport rows are rendered"
        );
        wheel(cx, -100_000.);
        paint(cx, &view, &rendered, width);
        assert!(
            rendered.borrow().contains(&255),
            "native wheel reaches the final row at width {width}: {:?}",
            rendered.borrow()
        );
        assert!(
            rendered.borrow().len() < 16,
            "the end is reached without eager measurement"
        );
        wheel(cx, 100_000.);
    }
}
