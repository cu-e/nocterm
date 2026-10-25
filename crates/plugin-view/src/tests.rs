use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, Bounds, FocusHandle, Hsla, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Pixels,
    TestAppContext, VisualTestContext, WindowHandle, hsla, px,
};
use nocterm_plugin_api::{Event, Node, NodeKind, PluginIcon, Tone, View};

use super::*;

#[derive(Clone, Default)]
struct Harness {
    seen: Rc<RefCell<Vec<Interaction>>>,
    handles: Rc<RefCell<Vec<FocusHandle>>>,
}

impl Harness {
    fn hooks(&self) -> Hooks {
        let seen = self.seen.clone();
        let handles = self.handles.clone();
        Hooks {
            icon: Rc::new(|_: PluginIcon| gpui::div().into_any_element()),
            event: Rc::new(move |interaction, _, _| seen.borrow_mut().push(interaction)),
            handles: Rc::new(move |slot, cx: &mut App| {
                let mut handles = handles.borrow_mut();
                while handles.len() <= slot {
                    handles.push(cx.focus_handle().tab_stop(true));
                }
                handles[slot].clone()
            }),
        }
    }

    fn seen(&self) -> Vec<Interaction> {
        self.seen.borrow().clone()
    }
}

fn style() -> ViewStyle {
    let colour = |hue: f32| -> Hsla { hsla(hue, 0.6, 0.5, 1.) };
    ViewStyle {
        text: colour(0.1),
        muted: colour(0.2),
        success: colour(0.3),
        caution: colour(0.4),
        border: colour(0.5),
        hover: colour(0.6),
        marker: colour(0.7),
    }
}

fn node(kind: NodeKind) -> Node {
    Node::new(kind)
}

fn child(parent: u16, kind: NodeKind) -> Node {
    Node {
        parent: Some(parent),
        ..Node::new(kind)
    }
}

fn keyed(parent: Option<u16>, key: &str, kind: NodeKind) -> Node {
    Node {
        parent,
        key: Some(key.into()),
        ..Node::new(kind)
    }
}

fn view(nodes: Vec<Node>) -> View {
    View {
        nodes,
        refresh: None,
    }
}

fn open(
    cx: &mut TestAppContext,
    kind: FrameKind,
    shown: View,
) -> (Harness, WindowHandle<PluginView>, &mut VisualTestContext) {
    let harness = Harness::default();
    let hooks = harness.hooks();
    let window = cx.add_window(|_, _| {
        let mut plugin = PluginView::new("Clock", kind, style(), hooks);
        let _ = plugin.show(shown);
        plugin
    });
    let cx = VisualTestContext::from_window(*window, cx).into_mut();
    cx.run_until_parked();
    (harness, window, cx)
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Option<Bounds<Pixels>> {
    cx.debug_bounds(selector)
}

fn centre(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Point<Pixels> {
    bounds(cx, selector)
        .unwrap_or_else(|| panic!("{selector} was not drawn"))
        .center()
}

#[gpui::test]
fn a_panel_draws_the_marker_and_the_label(cx: &mut TestAppContext) {
    let (_, window, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![node(NodeKind::Text("12:00".into()))]),
    );
    assert!(bounds(cx, "plugin-marker").is_some());
    assert!(bounds(cx, "plugin-label").is_some());
    let label = window.read_with(cx, |plugin, _| plugin.label()).unwrap();
    assert_eq!(label.as_ref(), "Plugin \u{b7} Clock");
}

#[gpui::test]
fn a_developer_session_adds_dev_to_the_label(cx: &mut TestAppContext) {
    let (_, window, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![node(NodeKind::Text("12:00".into()))]),
    );
    window.update(cx, |plugin, _, _| plugin.mark_dev()).unwrap();
    cx.run_until_parked();
    let label = window.read_with(cx, |plugin, _| plugin.label()).unwrap();
    assert_eq!(label.as_ref(), "Plugin \u{b7} Clock \u{b7} DEV");
    assert!(bounds(cx, "plugin-label").is_some());
}

#[gpui::test]
fn a_bar_widget_draws_the_marker_and_leaves_the_label_to_the_application(cx: &mut TestAppContext) {
    let (_, _, cx) = open(
        cx,
        FrameKind::Bar,
        view(vec![node(NodeKind::Text("12:00".into()))]),
    );
    assert!(bounds(cx, "plugin-marker").is_some());
    assert!(bounds(cx, "plugin-label").is_none());
}

#[gpui::test]
fn a_bar_widget_of_a_developer_session_draws_a_dev_chip_and_an_installed_one_does_not(
    cx: &mut TestAppContext,
) {
    let (_, window, cx) = open(
        cx,
        FrameKind::Bar,
        view(vec![node(NodeKind::Text("12:00".into()))]),
    );
    assert!(
        bounds(cx, "plugin-dev").is_none(),
        "an installed plugin has no DEV chip"
    );

    window
        .update(cx, |plugin, _, cx| {
            plugin.mark_dev();
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(
        bounds(cx, "plugin-dev").is_some(),
        "the developer session's widget does"
    );
    assert!(bounds(cx, "plugin-marker").is_some());
}

#[gpui::test]
fn a_refused_view_draws_a_fault_chip_of_fixed_width(cx: &mut TestAppContext) {
    let harness = Harness::default();
    let hooks = harness.hooks();
    let window = cx.add_window(|_, _| PluginView::new("Clock", FrameKind::Bar, style(), hooks));
    let cx = VisualTestContext::from_window(*window, cx).into_mut();
    let refused = view(vec![child(3, NodeKind::Spacer)]);
    let outcome = window
        .update(cx, |plugin, _, cx| {
            let outcome = plugin.show(refused);
            cx.notify();
            outcome
        })
        .unwrap();
    cx.run_until_parked();
    assert!(outcome.is_err());
    assert!(window.read_with(cx, |plugin, _| plugin.is_fault()).unwrap());
    let chip = bounds(cx, "plugin-fault").expect("the chip is drawn");
    assert_eq!(chip.size.width, px(72.));
}

#[gpui::test]
fn a_click_on_a_button_reports_its_key_as_a_pointer_gesture(cx: &mut TestAppContext) {
    let (harness, _, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![keyed(None, "go", NodeKind::Button("Go".into()))]),
    );
    let at = centre(cx, "plugin-control-0");
    cx.simulate_click(at, Default::default());
    assert_eq!(
        harness.seen(),
        vec![Interaction {
            event: Event::Activated { key: "go".into() },
            origin: Origin::Pointer,
        }]
    );
}

#[gpui::test]
fn enter_on_a_focused_button_reports_a_key_gesture(cx: &mut TestAppContext) {
    let (harness, _, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![keyed(None, "go", NodeKind::Button("Go".into()))]),
    );
    let handle = harness.handles.borrow()[0].clone();
    cx.update(|window, cx| {
        handle.focus(window, cx);
        window.refresh();
    });
    cx.run_until_parked();
    let keystroke = Keystroke::parse("enter").unwrap();
    cx.simulate_event(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke });
    assert_eq!(
        harness.seen(),
        vec![Interaction {
            event: Event::Activated { key: "go".into() },
            origin: Origin::Key,
        }]
    );
}

#[gpui::test]
fn a_click_on_a_toggle_reports_the_opposite_state(cx: &mut TestAppContext) {
    let (harness, _, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![keyed(None, "wrap", NodeKind::Toggle(true))]),
    );
    let at = centre(cx, "plugin-control-0");
    cx.simulate_click(at, Default::default());
    assert_eq!(
        harness.seen()[0].event,
        Event::Toggled {
            key: "wrap".into(),
            on: false
        }
    );
}

#[gpui::test]
fn drawing_controls_leaves_focus_where_it_was(cx: &mut TestAppContext) {
    let (_, _, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![
            node(NodeKind::Column),
            keyed(Some(0), "a", NodeKind::Button("A".into())),
            keyed(Some(0), "b", NodeKind::Toggle(false)),
        ]),
    );
    assert!(cx.update(|window, cx| window.focused(cx)).is_none());
}

#[gpui::test]
fn a_view_at_the_node_limit_draws(cx: &mut TestAppContext) {
    let mut nodes = vec![node(NodeKind::Column)];
    nodes.extend((1..nocterm_plugin_api::MAX_NODES).map(|_| child(0, NodeKind::Text("x".into()))));
    let (_, window, cx) = open(cx, FrameKind::Panel, view(nodes));
    assert!(!window.read_with(cx, |plugin, _| plugin.is_fault()).unwrap());
}

#[gpui::test]
fn every_node_kind_and_tone_draws_without_a_fault(cx: &mut TestAppContext) {
    let mut nodes = vec![node(NodeKind::Column), child(0, NodeKind::Row)];
    for tone in Tone::ALL {
        nodes.push(Node {
            parent: Some(1),
            tone,
            ..Node::new(NodeKind::Text("t".into()))
        });
    }
    nodes.extend([
        child(0, NodeKind::Icon(PluginIcon::Gem)),
        child(0, NodeKind::Meter(40)),
        child(0, NodeKind::Badge("new".into())),
        child(0, NodeKind::Divider),
        child(0, NodeKind::Spacer),
        child(0, NodeKind::List),
    ]);
    let list = u16::try_from(nodes.len() - 1).unwrap();
    nodes.push(child(list, NodeKind::Text("item".into())));
    let (_, window, cx) = open(cx, FrameKind::Panel, view(nodes));
    assert!(!window.read_with(cx, |plugin, _| plugin.is_fault()).unwrap());
}

#[gpui::test]
fn a_view_shown_later_replaces_the_fault(cx: &mut TestAppContext) {
    let (_, window, cx) = open(cx, FrameKind::Bar, view(vec![child(5, NodeKind::Spacer)]));
    assert!(bounds(cx, "plugin-fault").is_some());
    window
        .update(cx, |plugin, _, cx| {
            plugin
                .show(view(vec![node(NodeKind::Text("ok".into()))]))
                .unwrap();
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(bounds(cx, "plugin-fault").is_none());
}
