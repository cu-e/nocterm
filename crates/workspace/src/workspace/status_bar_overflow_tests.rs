//! A bar too narrow for its entries moves the least important into the `»` menu, and the menu
//! lists and activates them. Pinned entries and entries that ask for the user stay.
use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    Anchor, AppContext as _, Bounds, Context, ElementId, Entity, IntoElement, Pixels, Render,
    TestAppContext, TestSupportExt as _, Window,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        popover::Popover,
    },
    div,
    prelude::*,
    px, size,
    test::TestWindowExt as _,
};

use super::{
    WidgetRegistration, Workspace,
    status_bar_centre_tests::{Bar, bar_of, layout, say_much},
};
use crate::test_support::{EXPANDABLE, RIGHT_WIDGET, Toggle, toggle_id};

/// A widget of a width that tests can rely on.
struct Block {
    name: &'static str,
    width: f32,
}
impl Render for Block {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(self.name)
            .test_support()
            .w(px(self.width))
            .h(px(20.))
    }
}

/// A widget whose summary opens a popover that holds a button, as the host monitor's does.
struct Detailed {
    pressed: Rc<Cell<u32>>,
}
impl Render for Detailed {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pressed = self.pressed.clone();
        Popover::new("detailed-popover")
            .anchor(Anchor::BottomLeft)
            .trigger(
                Button::new("detailed-summary")
                    .ghost()
                    .small()
                    .label("Details"),
            )
            .content(move |_, _, _| {
                let pressed = pressed.clone();
                // Wider than the menu, as the host monitor's details are, and opening towards
                // the inside of the window: the button is beyond the menu's left edge.
                div().w(px(400.)).flex().justify_start().child(
                    Button::new("detailed-button")
                        .label("Press")
                        .on_click(move |_, _, _| pressed.set(pressed.get() + 1)),
                )
            })
    }
}

/// A flat bar: two toggles, then two widgets, the notices, the terminal and the side panel.
fn flat(cx: &mut TestAppContext, width: f32) -> Bar {
    flat_with(cx, width, |cx| {
        cx.new(|_| Block {
            name: "right-marker",
            width: 120.,
        })
    })
}

/// The flat bar with `widget` as its right-hand widget.
fn flat_with<V: Render>(
    cx: &mut TestAppContext,
    width: f32,
    widget: impl Fn(&mut Context<Workspace>) -> Entity<V> + 'static,
) -> Bar {
    let layouts = layout(
        serde_json::json!({
            "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", "test.panel-b"],
            }],
            "center": [],
            "right": [
                { "item": "workspace.notifications" },
                { "item": "test.widget" },
                { "item": "test.right-widget" },
                { "item": "workspace.local-terminal" },
                { "group": "assistant", "members": ["test.expandable"] },
            ],
        }),
        &[],
    );
    let bar = bar_of(cx, width, layouts);
    add_right_widget(cx, &bar, widget);
    settle(cx, &bar);
    bar
}

/// A narrow flat bar that also holds a notice: everything that can leave the bar has.
fn crowded(cx: &mut TestAppContext) -> Bar {
    let bar = flat(cx, 200.);
    cx.update_window(bar.handle, |_, window, cx| {
        say_much(window, cx);
        bar.workspace.update(cx, |_, cx| cx.notify());
    })
    .unwrap();
    settle(cx, &bar);
    bar
}

fn add_right_widget<V: Render>(
    cx: &mut TestAppContext,
    bar: &Bar,
    widget: impl Fn(&mut Context<Workspace>) -> Entity<V> + 'static,
) {
    cx.update_window(bar.handle, |_, window, cx| {
        bar.workspace.update(cx, |workspace, cx| {
            workspace
                .register_widget(
                    WidgetRegistration::new(RIGHT_WIDGET, move |_, _, cx| widget(cx)),
                    window,
                    cx,
                )
                .expect("the widget is in the test catalog");
        });
    })
    .unwrap();
}

/// Draws until the measures of a frame have been applied to the next.
fn settle(cx: &mut TestAppContext, bar: &Bar) {
    for _ in 0..4 {
        cx.update_window(bar.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn resize(cx: &mut TestAppContext, bar: &Bar, width: f32) {
    cx.simulate_window_resize(bar.handle, size(px(width), px(700.)));
    settle(cx, bar);
}

fn shown(cx: &mut TestAppContext, bar: &Bar, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(bar.handle, |_, window, _| window.try_find(id).is_some())
        .unwrap()
}

fn bounds(cx: &mut TestAppContext, bar: &Bar, id: impl Into<ElementId>) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(bar.handle, |_, window, _| window.find(id).bounds())
        .unwrap()
}

fn item(id: &str) -> ElementId {
    (ElementId::from("overflow-item"), id.to_owned()).into()
}

fn open_menu(cx: &mut TestAppContext, bar: &Bar) {
    cx.update_window(bar.handle, |_, window, cx| {
        window.click("status-overflow", cx)
    })
    .unwrap();
    settle(cx, bar);
}

/// Every entry of the bar that is drawn lies whole inside the footer's padding.
fn assert_nothing_clipped(cx: &mut TestAppContext, bar: &Bar, ids: &[ElementId]) {
    let footer = bounds(cx, bar, "workspace-footer");
    for id in ids {
        if shown(cx, bar, id.clone()) {
            let at = bounds(cx, bar, id.clone());
            assert!(
                at.left() >= footer.left() + px(7.5) && at.right() <= footer.right() - px(7.5),
                "{id:?} at {at:?} is clipped by {footer:?}"
            );
        }
    }
}

/// Where each drawn entry of the bar is.
fn snapshot(cx: &mut TestAppContext, bar: &Bar) -> Vec<(ElementId, Bounds<Pixels>)> {
    let mut drawn = Vec::new();
    for id in drawn_ids() {
        if shown(cx, bar, id.clone()) {
            drawn.push((id.clone(), bounds(cx, bar, id)));
        }
    }
    drawn
}

fn drawn_ids() -> Vec<ElementId> {
    vec![
        toggle_id(Toggle::Navigation(0)),
        toggle_id(Toggle::Navigation(1)),
        "centre-marker".into(),
        "right-marker".into(),
        toggle_id(Toggle::LocalTerminal),
        toggle_id(Toggle::SidePanel),
        "status-overflow".into(),
    ]
}

#[gpui_kit::test]
fn a_bar_with_room_has_no_overflow_and_settling_moves_nothing(cx: &mut TestAppContext) {
    let bar = flat(cx, 1280.);
    assert!(!shown(cx, &bar, "status-overflow"));
    let before = snapshot(cx, &bar);
    assert_eq!(before.len(), 6, "toggles, widgets and the side toggle");
    settle(cx, &bar);
    for (id, was) in before {
        assert_eq!(bounds(cx, &bar, id.clone()), was, "{id:?}");
    }
}

#[gpui_kit::test]
fn the_last_widget_goes_first_and_the_rest_stay_whole(cx: &mut TestAppContext) {
    let bar = flat(cx, 400.);
    assert!(shown(cx, &bar, "status-overflow"));
    assert!(
        !shown(cx, &bar, "right-marker"),
        "the later widget is hidden"
    );
    assert!(shown(cx, &bar, "centre-marker"), "the earlier one stays");
    for toggle in [
        Toggle::Navigation(0),
        Toggle::Navigation(1),
        Toggle::LocalTerminal,
        Toggle::SidePanel,
    ] {
        assert!(shown(cx, &bar, toggle_id(toggle)), "{toggle:?} stays");
    }
    assert_nothing_clipped(cx, &bar, &drawn_ids());
}

#[gpui_kit::test]
fn a_narrower_bar_hides_widgets_then_the_terminal_then_toggles_from_the_end(
    cx: &mut TestAppContext,
) {
    let bar = flat(cx, 300.);
    assert!(!shown(cx, &bar, "right-marker") && !shown(cx, &bar, "centre-marker"));
    assert!(shown(cx, &bar, toggle_id(Toggle::LocalTerminal)));
    resize(cx, &bar, 150.);
    assert!(!shown(cx, &bar, toggle_id(Toggle::LocalTerminal)));
    assert!(shown(cx, &bar, toggle_id(Toggle::Navigation(0))));
    assert_nothing_clipped(cx, &bar, &drawn_ids());
    resize(cx, &bar, 100.);
    assert!(
        !shown(cx, &bar, toggle_id(Toggle::Navigation(1))),
        "the later toggle goes before the earlier one"
    );
    assert_nothing_clipped(cx, &bar, &drawn_ids());
}

#[gpui_kit::test]
fn a_toggle_that_asks_for_the_user_and_the_notices_are_never_hidden(cx: &mut TestAppContext) {
    let bar = flat(cx, 100.);
    cx.update_window(bar.handle, |_, window, cx| {
        say_much(window, cx);
        bar.side_panel.update(cx, |panel, cx| {
            panel.set_attention(Some("permission".into()), cx)
        });
        bar.workspace.update(cx, |_, cx| cx.notify());
    })
    .unwrap();
    settle(cx, &bar);
    assert!(shown(cx, &bar, toggle_id(Toggle::SidePanel)));
    assert!(shown(cx, &bar, "notice-toggle"));
    assert!(shown(cx, &bar, "status-overflow"));
    assert!(!shown(cx, &bar, toggle_id(Toggle::Navigation(1))));
    let mut ids = drawn_ids();
    ids.push("notice-toggle".into());
    assert_nothing_clipped(cx, &bar, &ids);
}

#[gpui_kit::test]
fn the_menu_lists_hidden_widgets_and_activates_hidden_toggles(cx: &mut TestAppContext) {
    let bar = crowded(cx);
    assert!(!shown(cx, &bar, "status-overflow-list"));
    open_menu(cx, &bar);
    assert!(shown(cx, &bar, "status-overflow-list"));
    assert!(
        shown(cx, &bar, "right-marker"),
        "a widget keeps its own view"
    );
    assert!(shown(cx, &bar, "centre-marker"));
    assert!(shown(cx, &bar, item("workspace.local-terminal")));
    assert!(shown(cx, &bar, item("test.expandable")));
    assert!(
        shown(cx, &bar, toggle_id(Toggle::Navigation(0))) && !shown(cx, &bar, item("test.panel-a")),
        "an entry that stays is not listed"
    );

    cx.update_window(bar.handle, |_, window, cx| {
        window.click(item("test.expandable"), cx)
    })
    .unwrap();
    settle(cx, &bar);
    let opened = cx.update(|cx| bar.workspace.read(cx).panel_is_open(&EXPANDABLE.id));
    assert!(opened, "the row did what the toggle does");
    assert!(
        !shown(cx, &bar, "status-overflow-list"),
        "choosing a row closes the menu"
    );
}

#[gpui_kit::test]
fn the_menu_opens_the_local_terminal(cx: &mut TestAppContext) {
    let bar = crowded(cx);
    let opened = Rc::new(Cell::new(0));
    let counter = opened.clone();
    cx.update_window(bar.handle, |_, _, cx| {
        bar.workspace.update(cx, |workspace, _| {
            workspace.set_local_terminal_opener(move |_, _, _, _| counter.set(counter.get() + 1));
        });
    })
    .unwrap();
    open_menu(cx, &bar);
    cx.update_window(bar.handle, |_, window, cx| {
        window.click(item("workspace.local-terminal"), cx)
    })
    .unwrap();
    settle(cx, &bar);
    assert_eq!(opened.get(), 1, "the row did what the toggle does");
}

#[gpui_kit::test]
fn widening_the_window_brings_everything_back(cx: &mut TestAppContext) {
    let bar = flat(cx, 150.);
    assert!(shown(cx, &bar, "status-overflow"));
    resize(cx, &bar, 1280.);
    assert!(!shown(cx, &bar, "status-overflow"));
    for id in ["centre-marker", "right-marker"] {
        assert!(shown(cx, &bar, id), "{id}");
    }
    assert!(shown(cx, &bar, toggle_id(Toggle::LocalTerminal)));
    assert_nothing_clipped(cx, &bar, &drawn_ids());
}

#[gpui_kit::test]
fn a_settled_narrow_bar_stays_where_it_is(cx: &mut TestAppContext) {
    let bar = flat(cx, 300.);
    let before = snapshot(cx, &bar);
    settle(cx, &bar);
    settle(cx, &bar);
    for (id, was) in before {
        assert_eq!(bounds(cx, &bar, id.clone()), was, "{id:?} did not move");
    }
}

#[gpui_kit::test]
fn a_centre_that_does_not_fit_goes_into_the_menu_at_the_right_slot(cx: &mut TestAppContext) {
    let layouts = layout(
        serde_json::json!({
            "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", "test.panel-b"],
            }],
            "center": [{ "item": "test.widget" }],
            "right": [
                { "item": "workspace.notifications" },
                { "group": "assistant", "members": ["test.expandable"] },
            ],
        }),
        &[],
    );
    let bar = bar_of(cx, 250., layouts);
    settle(cx, &bar);
    assert!(
        !shown(cx, &bar, "centre-marker"),
        "the centre widget gave way"
    );
    let button = bounds(cx, &bar, "status-overflow");
    let side = bounds(cx, &bar, toggle_id(Toggle::SidePanel));
    assert!(button.right() <= side.left(), "{button:?} before {side:?}");
    assert_nothing_clipped(cx, &bar, &drawn_ids());
    open_menu(cx, &bar);
    assert!(shown(cx, &bar, "centre-marker"), "the menu lists it");
}

/// A bar whose `Detailed` widget is in the menu, the menu open, and the widget's popover open.
fn popover_open_from_the_menu(cx: &mut TestAppContext) -> (Bar, Rc<Cell<u32>>) {
    let pressed = Rc::new(Cell::new(0));
    let widget = pressed.clone();
    let bar = flat_with(cx, 340., move |cx| {
        let pressed = widget.clone();
        cx.new(|_| Detailed { pressed })
    });
    assert!(!shown(cx, &bar, "detailed-summary"), "the widget is hidden");
    open_menu(cx, &bar);
    assert!(shown(cx, &bar, "detailed-summary"), "the menu lists it");
    click(cx, &bar, "detailed-summary");
    assert!(shown(cx, &bar, "detailed-button"), "the popover is open");
    (bar, pressed)
}

fn click(cx: &mut TestAppContext, bar: &Bar, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(bar.handle, |_, window, cx| window.click(id, cx))
        .unwrap();
    settle(cx, bar);
}

#[gpui_kit::test]
fn a_button_inside_a_widgets_popover_works_while_the_widget_is_in_the_menu(
    cx: &mut TestAppContext,
) {
    let (bar, pressed) = popover_open_from_the_menu(cx);

    click(cx, &bar, "detailed-button");

    assert_eq!(pressed.get(), 1, "the press reached the button");
    assert!(shown(cx, &bar, "status-overflow-list"), "the menu stayed");
}

#[gpui_kit::test]
fn a_press_outside_the_menu_closes_it(cx: &mut TestAppContext) {
    let bar = crowded(cx);
    open_menu(cx, &bar);
    assert!(shown(cx, &bar, "status-overflow-list"));

    click(cx, &bar, toggle_id(Toggle::Navigation(0)));

    assert!(!shown(cx, &bar, "status-overflow-list"), "the menu closed");
}

#[gpui_kit::test]
fn a_press_outside_closes_the_widgets_popover_first_and_then_the_menu(cx: &mut TestAppContext) {
    let (bar, _) = popover_open_from_the_menu(cx);

    click(cx, &bar, toggle_id(Toggle::Navigation(0)));
    assert!(!shown(cx, &bar, "detailed-button"), "the popover closed");
    assert!(shown(cx, &bar, "status-overflow-list"), "the menu stayed");

    click(cx, &bar, toggle_id(Toggle::Navigation(0)));
    assert!(!shown(cx, &bar, "status-overflow-list"), "the menu closed");
}
