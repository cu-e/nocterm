use alloc::{string::ToString, vec::Vec};

use proptest::prelude::*;

use crate::{
    LimitError, MAX_DEPTH, MAX_LIST_ITEMS, MAX_NODES, MIN_REFRESH_MS, Node, NodeKind, Refresh,
    Tone, View, ViewError,
};

fn node(parent: Option<u16>, kind: NodeKind) -> Node {
    Node {
        parent,
        ..Node::new(kind)
    }
}

fn keyed(key: &str, kind: NodeKind) -> Node {
    Node {
        key: Some(key.to_string()),
        ..Node::new(kind)
    }
}

fn view(nodes: Vec<Node>) -> View {
    View {
        nodes,
        refresh: None,
    }
}

#[test]
fn a_small_tree_is_valid() {
    let tree = view(Vec::from([
        Node::new(NodeKind::Row),
        node(Some(0), NodeKind::Icon(crate::Glyph::Puzzle)),
        node(Some(0), NodeKind::Text("12:00".into())),
        keyed("go", NodeKind::Button("Go".into())),
        keyed("on", NodeKind::Toggle(true)),
        Node {
            tone: Tone::Caution,
            ..Node::new(NodeKind::Meter(100))
        },
    ]));
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn parents_must_come_earlier_and_hold_children() {
    let forward = view(Vec::from([
        node(Some(1), NodeKind::Row),
        Node::new(NodeKind::Row),
    ]));
    assert_eq!(forward.validate(), Err(ViewError::Parent { index: 0 }));
    let itself = view(Vec::from([node(Some(0), NodeKind::Row)]));
    assert_eq!(itself.validate(), Err(ViewError::Parent { index: 0 }));
    let leaf = view(Vec::from([
        Node::new(NodeKind::Text("a".into())),
        node(Some(0), NodeKind::Spacer),
    ]));
    assert_eq!(leaf.validate(), Err(ViewError::NotAContainer { index: 1 }));
}

#[test]
fn depth_is_limited_to_eight_levels() {
    let chain = |levels: usize| {
        view(
            (0..levels)
                .map(|i| node(i.checked_sub(1).map(|p| p as u16), NodeKind::Column))
                .collect(),
        )
    };
    assert_eq!(chain(usize::from(MAX_DEPTH)).validate(), Ok(()));
    assert_eq!(
        chain(usize::from(MAX_DEPTH) + 1).validate(),
        Err(ViewError::TooDeep {
            index: usize::from(MAX_DEPTH)
        })
    );
}

#[test]
fn size_limits_apply_to_nodes_and_list_items() {
    let wide = view(
        (0..=MAX_NODES)
            .map(|_| Node::new(NodeKind::Spacer))
            .collect(),
    );
    assert_eq!(wide.validate(), Err(ViewError::TooManyNodes));
    let list = |items: usize| {
        let mut nodes = Vec::from([Node::new(NodeKind::List)]);
        nodes.extend((0..items).map(|_| node(Some(0), NodeKind::Spacer)));
        view(nodes)
    };
    assert_eq!(list(MAX_LIST_ITEMS).validate(), Ok(()));
    assert_eq!(
        list(MAX_LIST_ITEMS + 1).validate(),
        Err(ViewError::TooManyItems)
    );
}

#[test]
fn keys_are_required_for_controls_and_unique() {
    let none = view(Vec::from([Node::new(NodeKind::Button("Go".into()))]));
    assert_eq!(none.validate(), Err(ViewError::MissingKey { index: 0 }));
    let twice = view(Vec::from([
        keyed("a", NodeKind::Button("A".into())),
        keyed("a", NodeKind::Toggle(false)),
    ]));
    assert_eq!(
        twice.validate(),
        Err(ViewError::DuplicateKey { key: "a".into() })
    );
    let bad = view(Vec::from([keyed("A key", NodeKind::Button("A".into()))]));
    assert!(matches!(
        bad.validate(),
        Err(ViewError::Limit {
            source: LimitError::Syntax { .. },
            ..
        })
    ));
}

#[test]
fn strings_are_bounded_and_refreshes_are_not_too_fast() {
    let long = view(Vec::from([Node::new(NodeKind::Badge("123456789".into()))]));
    assert!(matches!(
        long.validate(),
        Err(ViewError::Limit {
            source: LimitError::TooLong { max: 8, .. },
            ..
        })
    ));
    let meter = view(Vec::from([Node::new(NodeKind::Meter(101))]));
    assert!(matches!(
        meter.validate(),
        Err(ViewError::Limit {
            source: LimitError::Range { .. },
            ..
        })
    ));
    let fast = View {
        refresh: Some(Refresh::AfterMs(MIN_REFRESH_MS - 1)),
        ..View::default()
    };
    assert_eq!(fast.validate(), Err(ViewError::Refresh));
    let slow = View {
        refresh: Some(Refresh::AfterMs(MIN_REFRESH_MS)),
        ..View::default()
    };
    assert_eq!(slow.validate(), Ok(()));
}

#[test]
fn guest_text_is_sanitized_before_validation_and_refused_without_it() {
    let hostile = view(Vec::from([Node::new(NodeKind::Text(
        "ok\u{202E}evil".into(),
    ))]));
    assert!(matches!(
        hostile.validate(),
        Err(ViewError::Limit {
            source: LimitError::Unsanitized { .. },
            ..
        })
    ));
    let cleaned = hostile.into_validated().unwrap();
    assert_eq!(cleaned.nodes[0].kind, NodeKind::Text("okevil".into()));
}

fn arbitrary_node() -> impl Strategy<Value = Node> {
    let kind = prop_oneof![
        Just(NodeKind::Row),
        Just(NodeKind::Column),
        Just(NodeKind::List),
        Just(NodeKind::Spacer),
        Just(NodeKind::Divider),
        "[a-z]{0,12}".prop_map(NodeKind::Text),
        "[a-z]{0,12}".prop_map(NodeKind::Button),
        any::<bool>().prop_map(NodeKind::Toggle),
        any::<u8>().prop_map(NodeKind::Meter),
    ];
    (
        proptest::option::of(0u16..40),
        proptest::option::of(prop_oneof![
            Just("a".to_string()),
            Just("b".to_string()),
            "[a-z]{1,4}".prop_map(|s| s)
        ]),
        kind,
    )
        .prop_map(|(parent, key, kind)| Node {
            parent,
            key,
            kind,
            tone: Tone::Default,
        })
}

proptest! {
    #[test]
    fn a_valid_arena_satisfies_every_rule(nodes in proptest::collection::vec(arbitrary_node(), 0..40)) {
        let candidate = view(nodes);
        if candidate.validate().is_ok() {
            let mut depth = alloc::vec![1u8; candidate.nodes.len()];
            let mut keys = alloc::collections::BTreeSet::new();
            prop_assert!(candidate.nodes.len() <= MAX_NODES);
            for (index, node) in candidate.nodes.iter().enumerate() {
                if let Some(parent) = node.parent {
                    let parent = usize::from(parent);
                    prop_assert!(parent < index);
                    prop_assert!(matches!(candidate.nodes[parent].kind, NodeKind::Row | NodeKind::Column | NodeKind::List));
                    depth[index] = depth[parent] + 1;
                    prop_assert!(depth[index] <= MAX_DEPTH);
                }
                if let Some(key) = &node.key {
                    prop_assert!(keys.insert(key.clone()));
                }
                if matches!(node.kind, NodeKind::Button(_) | NodeKind::Toggle(_)) {
                    prop_assert!(node.key.is_some());
                }
            }
        }
    }
}

#[test]
fn a_code_block_keeps_its_lines_and_drops_what_would_not_show_as_text() {
    let mut shown = view(Vec::from([Node::new(NodeKind::Code(
        "server.properties:\n\tmotd=Hi\u{202e}\nport=25565\u{7}".into(),
    ))]));
    assert!(
        shown.validate().is_err(),
        "a guest's text is checked as it is"
    );
    shown.sanitize();
    assert_eq!(shown.validate(), Ok(()));
    assert_eq!(
        shown.nodes[0].kind,
        NodeKind::Code("server.properties:\n    motd=Hi\nport=25565".into())
    );
    let long = view(Vec::from([Node::new(NodeKind::Code(
        "x".repeat(crate::MAX_CODE + 1),
    ))]));
    assert!(matches!(
        long.validate(),
        Err(ViewError::Limit {
            source: LimitError::TooLong { field: "code", .. },
            ..
        })
    ));
    let mut cut = long.clone();
    cut.sanitize();
    assert_eq!(cut.validate(), Ok(()));
}

#[test]
fn sections_and_items_hold_children_and_an_icon_button_needs_a_key() {
    let section = NodeKind::Section {
        title: "Servers".into(),
        count: Some("2".into()),
        folded: false,
    };
    let item = NodeKind::Item {
        title: "Build".into(),
        detail: Some("on".into()),
        icon: Some(crate::Glyph::Server),
        selected: true,
    };
    let button = NodeKind::IconButton {
        icon: crate::Glyph::Stop,
        label: "Stop".into(),
    };
    let ok = view(Vec::from([
        keyed("servers", section.clone()),
        Node {
            parent: Some(0),
            key: Some("open".into()),
            ..Node::new(item.clone())
        },
        Node {
            parent: Some(1),
            key: Some("stop".into()),
            ..Node::new(button.clone())
        },
        node(Some(0), NodeKind::Caption("one is off".into())),
        node(
            Some(0),
            NodeKind::Field {
                label: "CPU".into(),
                value: "38%".into(),
            },
        ),
    ]));
    assert_eq!(ok.validate(), Ok(()));
    let keyless = view(Vec::from([Node::new(item), node(Some(0), button)]));
    assert_eq!(keyless.validate(), Err(ViewError::MissingKey { index: 1 }));
    let leaf = view(Vec::from([
        Node::new(NodeKind::Caption("a".into())),
        node(Some(0), NodeKind::Spacer),
    ]));
    assert_eq!(leaf.validate(), Err(ViewError::NotAContainer { index: 1 }));
}

#[test]
fn component_text_is_sanitized_and_bounded() {
    let mut long = view(Vec::from([Node::new(NodeKind::Section {
        title: "a\u{202e}b".repeat(40),
        count: Some("123456789".into()),
        folded: false,
    })]));
    long.sanitize();
    let NodeKind::Section { title, count, .. } = &long.nodes[0].kind else {
        unreachable!()
    };
    assert!(title.chars().count() <= crate::MAX_BUTTON);
    assert!(!title.contains('\u{202e}'));
    assert_eq!(
        count.as_deref().map(|count| count.chars().count()),
        Some(crate::MAX_BADGE)
    );
    assert_eq!(long.validate(), Ok(()));
    let unsanitized = view(Vec::from([Node::new(NodeKind::Field {
        label: "CPU".into(),
        value: "\u{202e}".into(),
    })]));
    assert!(matches!(
        unsanitized.validate(),
        Err(ViewError::Limit {
            index: 0,
            source: LimitError::Unsanitized { .. }
        })
    ));
}

#[test]
fn a_chart_has_one_or_two_series_of_at_most_its_span_of_values() {
    use crate::{MAX_CHART_POINTS, MAX_CHART_SERIES};
    let chart = |series: Vec<Vec<u32>>, span: Option<u16>| {
        view(Vec::from([Node::new(NodeKind::Chart {
            series,
            max: Some(100),
            span,
        })]))
        .validate()
    };
    let values = |count: usize| (0..count as u32).collect::<Vec<_>>();
    assert_eq!(chart(Vec::from([values(3), values(3)]), Some(60)), Ok(()));
    assert_eq!(chart(Vec::from([values(MAX_CHART_POINTS)]), None), Ok(()));
    let refused = |result: Result<(), ViewError>| matches!(result, Err(ViewError::Limit { .. }));
    assert!(refused(chart(Vec::new(), None)), "a chart needs a series");
    assert!(refused(chart(
        (0..=MAX_CHART_SERIES).map(|_| values(2)).collect(),
        None
    )));
    assert!(refused(chart(
        Vec::from([values(MAX_CHART_POINTS + 1)]),
        None
    )));
    assert!(refused(chart(Vec::from([values(2)]), Some(0))));
    assert!(refused(chart(
        Vec::from([values(2)]),
        Some(MAX_CHART_POINTS as u16 + 1)
    )));
}
