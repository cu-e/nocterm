//! A widget that follows its zone is told where the bar places it, when it is created and when
//! a layout moves it, and the notice list opens toward the inside of the window.
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    AppContext as _, Context, IntoElement, Render, TestAppContext, TestSupportExt as _, Window,
    div, prelude::*, px, test::TestWindowExt as _,
};
use nocterm_plugin_model::Zone;
use nocterm_window_layout::{LayoutSettings, PresetId};

use super::{
    WidgetRegistration, Workspace, ZoneAware,
    status_bar_centre_tests::{Bar, bar_of, say_much},
};
use crate::test_support::RIGHT_WIDGET;

/// A widget that records the zones it is told about.
struct Follower {
    told: Rc<RefCell<Vec<Zone>>>,
}
impl Render for Follower {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("follower").test_support().w_8().h_4()
    }
}
impl ZoneAware for Follower {
    fn set_zone(&mut self, zone: Zone, _: &mut Context<Self>) {
        self.told.borrow_mut().push(zone);
    }
}

/// Two layouts that place the follower on the right and on the left.
fn layouts() -> LayoutSettings {
    let bar = |side: &str| {
        let mut bar = serde_json::json!({
            "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", "test.panel-b"],
            }],
            "center": [],
            "right": [{ "item": "workspace.notifications" }],
        });
        bar[side]
            .as_array_mut()
            .expect("a zone is a list")
            .push(serde_json::json!({ "item": "test.right-widget" }));
        bar
    };
    serde_json::from_value(serde_json::json!({
        "active": "on-right",
        "presets": [
            { "id": "on-right", "name": "On the right", "bar": bar("right"), "tray": [] },
            { "id": "on-left", "name": "On the left", "bar": bar("left"), "tray": [] },
        ],
    }))
    .expect("the stored layouts are valid")
}

fn follower_bar(cx: &mut TestAppContext, told: &Rc<RefCell<Vec<Zone>>>) -> Bar {
    let bar = bar_of(cx, 1280., layouts());
    let told = told.clone();
    cx.update_window(bar.handle, |_, window, cx| {
        bar.workspace.update(cx, |workspace, cx| {
            workspace
                .register_widget(
                    WidgetRegistration::zone_aware(RIGHT_WIDGET, move |_: &Workspace, _, cx| {
                        let told = told.clone();
                        cx.new(|_| Follower { told })
                    }),
                    window,
                    cx,
                )
                .expect("the widget is in the test catalog");
        });
    })
    .unwrap();
    cx.run_until_parked();
    bar
}

fn switch(cx: &mut TestAppContext, bar: &Bar, id: &'static str) {
    let task = cx
        .update_window(bar.handle, |_, _, cx| {
            bar.workspace.update(cx, |workspace, cx| {
                workspace.switch_layout(PresetId::from_owned(id.to_owned()), cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.foreground_executor()
        .block_test(task)
        .expect("the layout is selected");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_widget_is_told_the_zone_that_places_it(cx: &mut TestAppContext) {
    let told = Rc::new(RefCell::new(Vec::new()));
    let _bar = follower_bar(cx, &told);
    assert_eq!(*told.borrow(), [Zone::Right]);
}

#[gpui_kit::test]
fn a_layout_that_moves_the_widget_tells_it_the_new_zone_once(cx: &mut TestAppContext) {
    let told = Rc::new(RefCell::new(Vec::new()));
    let bar = follower_bar(cx, &told);
    switch(cx, &bar, "on-left");
    assert_eq!(*told.borrow(), [Zone::Right, Zone::Left]);
    cx.update_window(bar.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(
        told.borrow().len(),
        2,
        "drawing again does not tell it again"
    );
}

#[gpui_kit::test]
fn the_notice_list_opens_toward_the_inside_from_a_left_zone(cx: &mut TestAppContext) {
    let layouts = serde_json::from_value(serde_json::json!({
        "active": "p1",
        "presets": [{
            "id": "p1",
            "name": "Notices on the left",
            "bar": {
                "left": [
                    { "group": "navigation", "primary": true, "members": ["test.panel-a", "test.panel-b"] },
                    { "item": "workspace.notifications" },
                ],
                "center": [],
                "right": [],
            },
            "tray": [],
        }],
    }))
    .expect("the stored layout is valid");
    let bar = bar_of(cx, 1280., layouts);
    cx.update_window(bar.handle, |_, window, cx| {
        say_much(window, cx);
        window.render_frame(cx);
        window.click("notice-toggle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(bar.handle, |_, window, cx| {
        window.render_frame(cx);
        let bell = window.find("notice-toggle").bounds();
        let list = window.find("notice-list").bounds();
        assert!(
            list.left() >= bell.left() - px(1.),
            "the list {list:?} grows rightwards from the bell {bell:?}"
        );
    })
    .unwrap();
}
