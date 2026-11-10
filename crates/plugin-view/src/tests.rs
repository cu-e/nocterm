use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, Bounds, FocusHandle, Hsla, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Pixels,
    TestAppContext, VisualTestContext, WindowHandle, hsla, px,
};
use nocterm_plugin_api::{Event, Glyph, Node, NodeKind, Tone, View};

use super::*;

#[derive(Clone, Default)]
struct Harness {
    seen: Rc<RefCell<Vec<Interaction>>>,
    handles: Rc<RefCell<Vec<FocusHandle>>>,
    /// The view the redraw hook redraws, once it exists.
    shown: Rc<RefCell<Option<gpui::WeakEntity<PluginView>>>>,
    redraws: Rc<RefCell<usize>>,
}

impl Harness {
    fn hooks(&self) -> Hooks {
        let seen = self.seen.clone();
        let handles = self.handles.clone();
        Hooks {
            icon: Rc::new(|_: Glyph| gpui::div().into_any_element()),
            tooltip: Rc::new(|_, _, _| unreachable!("no test hovers a control")),
            redraw: {
                let (shown, redraws) = (self.shown.clone(), self.redraws.clone());
                Rc::new(move |cx: &mut App| {
                    *redraws.borrow_mut() += 1;
                    let view = shown.borrow().clone();
                    if let Some(view) = view.and_then(|view| view.upgrade()) {
                        view.update(cx, |_, cx| cx.notify());
                    }
                })
            },
            event: Rc::new(move |interaction, _, _| seen.borrow_mut().push(interaction)),
            handles: Rc::new(move |slot, cx: &mut App| {
                let mut handles = handles.borrow_mut();
                while handles.len() <= slot {
                    handles.push(cx.focus_handle().tab_stop(true));
                }
                handles[slot].clone()
            }),
            mono: "monospace".into(),
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
        selected: colour(0.65),
        marker: colour(0.7),
        chart: [colour(0.8), colour(0.9)],
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
    let root = window.root(cx).unwrap().downgrade();
    *harness.shown.borrow_mut() = Some(root);
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
fn a_panel_scrolls_its_body_and_leaves_its_label_to_the_shell(cx: &mut TestAppContext) {
    let (_, window, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![node(NodeKind::Text("12:00".into()))]),
    );
    assert!(bounds(cx, "plugin-panel").is_some());
    assert!(
        bounds(cx, "plugin-marker").is_none(),
        "the shell writes the provenance"
    );
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
        child(0, NodeKind::Icon(Glyph::Gem)),
        child(0, NodeKind::Meter(40)),
        child(0, NodeKind::Badge("new".into())),
        child(0, NodeKind::Divider),
        child(0, NodeKind::Code("one\ntwo".into())),
        child(0, NodeKind::Spacer),
        child(0, NodeKind::List),
    ]);
    let list = u16::try_from(nodes.len() - 1).unwrap();
    nodes.push(child(list, NodeKind::Text("item".into())));
    let section = u16::try_from(nodes.len()).unwrap();
    nodes.extend([
        keyed(None, "servers", section_kind("Servers", false)),
        child(section, NodeKind::Caption("none yet".into())),
        child(
            section,
            NodeKind::Field {
                label: "CPU".into(),
                value: "38%".into(),
            },
        ),
        child(section, item_kind("Build", true)),
    ]);
    let item = u16::try_from(nodes.len() - 1).unwrap();
    nodes.push(keyed(
        Some(item),
        "stop",
        NodeKind::IconButton {
            icon: Glyph::Stop,
            label: "Stop".into(),
        },
    ));
    let (_, window, cx) = open(cx, FrameKind::Panel, view(nodes.clone()));
    assert!(!window.read_with(cx, |plugin, _| plugin.is_fault()).unwrap());
    let (_, window, cx) = open(cx, FrameKind::Bar, view(nodes));
    assert!(!window.read_with(cx, |plugin, _| plugin.is_fault()).unwrap());
}

fn section_kind(title: &str, folded: bool) -> NodeKind {
    NodeKind::Section {
        title: title.into(),
        count: Some("2".into()),
        folded,
    }
}

fn item_kind(title: &str, selected: bool) -> NodeKind {
    NodeKind::Item {
        title: title.into(),
        detail: Some("on".into()),
        icon: Some(Glyph::Server),
        selected,
    }
}

/// A foldable section `servers` that holds one button, whose control shows whether the section
/// is open.
fn servers(folded: bool) -> View {
    view(vec![
        keyed(None, "servers", section_kind("Servers", folded)),
        keyed(Some(0), "go", NodeKind::Button("Go".into())),
    ])
}

fn show(window: WindowHandle<PluginView>, shown: View, cx: &mut VisualTestContext) {
    window
        .update(cx, |plugin, _, cx| {
            plugin.show(shown).unwrap();
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn the_user_folds_a_section_without_asking_the_plugin_and_the_fold_outlives_a_new_view(
    cx: &mut TestAppContext,
) {
    let (harness, window, cx) = open(cx, FrameKind::Panel, servers(false));
    assert!(
        bounds(cx, "plugin-control-0").is_some(),
        "an open section shows its content"
    );

    let heading = centre(cx, "plugin-section-0");
    cx.simulate_click(heading, Default::default());

    assert!(harness.seen().is_empty(), "folding is not a plugin event");
    assert_eq!(*harness.redraws.borrow(), 1);
    assert!(
        bounds(cx, "plugin-control-0").is_none(),
        "a folded section hides it"
    );
    show(window, servers(false), cx);
    assert!(
        bounds(cx, "plugin-control-0").is_none(),
        "the fold outlives a new view"
    );
    show(window, view(vec![node(NodeKind::Text("gone".into()))]), cx);
    show(window, servers(false), cx);
    assert!(
        bounds(cx, "plugin-control-0").is_some(),
        "a section that left the view starts again as the plugin asks"
    );
}

#[gpui::test]
fn a_section_the_plugin_starts_folded_shows_its_content_when_the_user_opens_it(
    cx: &mut TestAppContext,
) {
    let (_, _, cx) = open(cx, FrameKind::Panel, servers(true));
    assert!(bounds(cx, "plugin-control-0").is_none());
    let heading = centre(cx, "plugin-section-0");
    cx.simulate_click(heading, Default::default());
    assert!(bounds(cx, "plugin-control-0").is_some());
}

#[gpui::test]
fn a_click_on_an_icon_button_in_a_row_reports_only_the_button(cx: &mut TestAppContext) {
    let (harness, _, cx) = open(
        cx,
        FrameKind::Panel,
        view(vec![
            keyed(None, "open", item_kind("Build", false)),
            keyed(
                Some(0),
                "stop",
                NodeKind::IconButton {
                    icon: Glyph::Stop,
                    label: "Stop".into(),
                },
            ),
        ]),
    );
    let button = centre(cx, "plugin-control-1");
    cx.simulate_click(button, Default::default());
    let row = centre(cx, "plugin-control-0");
    cx.simulate_click(row, Default::default());

    let keys: Vec<Event> = harness.seen().into_iter().map(|seen| seen.event).collect();
    assert_eq!(
        keys,
        [
            Event::Activated { key: "stop".into() },
            Event::Activated { key: "open".into() }
        ]
    );
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

/// A text, a spacer and a button: the bar of a terminal tab.
fn terminal_bar() -> View {
    view(vec![
        node(NodeKind::Text("running".into())),
        node(NodeKind::Spacer),
        keyed(None, "restart", NodeKind::Button("Restart".into())),
    ])
}

#[gpui::test]
fn a_strip_fills_its_width_so_a_spacer_moves_the_controls_to_its_end(cx: &mut TestAppContext) {
    let (harness, _, cx) = open(cx, FrameKind::Strip, terminal_bar());
    let strip = bounds(cx, "plugin-strip").expect("the strip is drawn");
    let marker = bounds(cx, "plugin-marker").expect("the host's marker leads");
    let button = bounds(cx, "plugin-control-0").expect("the button is drawn");
    assert!(marker.left() < button.left());
    assert!(
        strip.right() - button.right() < px(24.),
        "the button sits at the end: {strip:?} {button:?}"
    );
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    assert_eq!(
        harness.seen().last().map(|seen| seen.event.clone()),
        Some(Event::Activated {
            key: "restart".into()
        })
    );
}

#[gpui::test]
fn a_rail_stacks_its_roots_under_the_marker(cx: &mut TestAppContext) {
    let (_, _, cx) = open(cx, FrameKind::Rail, terminal_bar());
    assert!(bounds(cx, "plugin-rail").is_some());
    let marker = bounds(cx, "plugin-marker").expect("the host's marker is on top");
    let button = bounds(cx, "plugin-control-0").expect("the button is drawn");
    assert!(marker.bottom() <= button.top(), "{marker:?} {button:?}");
}

fn charts() -> View {
    view(vec![node(NodeKind::Chart {
        series: vec![vec![10, 40, 30], vec![5, 5, 20]],
        max: Some(100),
        span: Some(60),
    })])
}

#[gpui::test]
fn a_chart_fills_a_panels_width_and_is_a_short_line_in_a_strip(cx: &mut TestAppContext) {
    let (_, _, cx) = open(cx, FrameKind::Panel, charts());
    let panel = bounds(cx, "plugin-chart").expect("the chart is drawn");
    assert!(panel.size.width > px(200.), "{panel:?}");
    assert!(panel.size.height >= px(40.), "{panel:?}");

    let (_, _, cx) = open(&mut cx.cx, FrameKind::Strip, charts());
    let strip = bounds(cx, "plugin-chart").expect("the chart is drawn");
    assert!(strip.size.width <= px(56.), "{strip:?}");
    assert!(strip.size.height <= px(16.), "{strip:?}");
}
