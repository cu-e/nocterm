use super::*;
use nocterm_ai::model::{
    content::{Content, Text},
    ids::ToolId,
    tools::{ToolKind, ToolRecord, ToolStatus},
};
use nocterm_ai::{testing::ManagedRows, thread::Entry};
fn text(text: &str) -> Content {
    Content::Text(Text {
        text: text.into(),
        annotations: None,
        extensions: None,
    })
}
fn tool(id: &str, title: &str) -> ToolRecord {
    ToolRecord {
        id: ToolId::new(id),
        title: title.into(),
        name: None,
        kind: ToolKind::Other,
        status: ToolStatus::Pending,
        output: Vec::new(),
        locations: Vec::new(),
        arguments: None,
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
        client_display: None,
    }
}

fn message(value: &str) -> Entry {
    Entry::User(vec![text(value)])
}
fn top(item_ix: usize, offset: f32) -> ListOffset {
    ListOffset {
        item_ix,
        offset_in_item: px(offset),
    }
}
fn mixed() -> Vec<Entry> {
    vec![
        Entry::Thought("before messages".into()),
        Entry::User(vec![text("question")]),
        Entry::Tool(tool("tool", "Inspect")),
        Entry::Thought("thinking".into()),
        Entry::Agent("answer".into()),
        Entry::Content(text("content")),
        message("final"),
    ]
}

#[test]
fn targets_use_user_messages_and_distinguish_partial_from_exact_starts() {
    let mut navigation = MessageNavigation::default();
    navigation.observe(1.into(), ManagedRows::new(mixed()).unwrap().rows(), &[]);
    assert_eq!(navigation.anchors, [1, 6]);
    for (viewport, expected) in [
        (top(0, 0.), None),
        (top(1, 0.), None),
        (top(1, 12.), Some(1)),
        (top(2, 0.), Some(1)),
        (top(3, 20.), Some(1)),
        (top(4, 0.), Some(1)),
        (top(4, 0.5), Some(1)),
        (top(5, 0.), Some(1)),
        (top(6, 0.), Some(1)),
        (top(6, 2.), Some(6)),
        (top(7, 0.), Some(6)),
    ] {
        assert_eq!(navigation.previous(viewport), expected, "{viewport:?}");
    }
    assert_eq!(navigation.first(top(0, 0.)), None);
    assert_eq!(navigation.first(top(0, 5.)), Some(0));
    assert_eq!(navigation.first(top(1, 0.)), Some(0));
    assert_eq!(navigation.first(top(6, 0.)), Some(0));
}

#[test]
fn cache_refreshes_only_appended_and_dirty_classifications() {
    let mut entries = (0..10_000)
        .map(|i| {
            if i % 2 == 0 {
                message("answer")
            } else {
                Entry::Thought("thinking".into())
            }
        })
        .collect::<Vec<_>>();
    let mut navigation = MessageNavigation::default();
    let mut rows = ManagedRows::new(entries.clone()).unwrap();
    navigation.observe(1.into(), rows.rows(), &[]);
    assert_eq!(navigation.inspections, 10_000);
    navigation.observe(1.into(), rows.rows(), &[]);
    assert_eq!(
        navigation.inspections, 10_000,
        "unchanged frame walked history"
    );
    entries[9998] = message("streamed text grows without changing classification");
    rows.replace(entries.clone()).unwrap();
    navigation.observe(1.into(), rows.rows(), &[9998]);
    assert_eq!(navigation.inspections, 10_001);
    entries.push(Entry::Tool(tool("new", "Tool")));
    entries.push(message("new answer"));
    entries[0] = Entry::Thought("old row changes classification".into());
    rows.replace(entries).unwrap();
    navigation.observe(1.into(), rows.rows(), &[0, 10_001, usize::MAX]);
    assert_eq!(
        navigation.inspections, 10_004,
        "append and one dirty row must inspect three entries"
    );
    assert_eq!(navigation.anchors.first(), Some(&2));
    assert_eq!(navigation.anchors.last(), Some(&10_001));
    let before = navigation.inspections;
    for _ in 0..1_000 {
        assert_eq!(navigation.previous(top(10_001, 0.)), Some(9998));
        assert_eq!(navigation.first(top(10_001, 0.)), Some(0));
    }
    assert_eq!(
        navigation.inspections, before,
        "navigation walked transcript content"
    );
}

#[test]
fn dirty_rows_can_become_messages_without_duplicates_or_stale_anchors() {
    let mut entries = mixed();
    let mut navigation = MessageNavigation::default();
    let mut rows = ManagedRows::new(entries.clone()).unwrap();
    navigation.observe(1.into(), rows.rows(), &[]);
    entries[2] = message("tool row replaced by restored answer");
    entries[4] = Entry::Thought("answer row replaced by thought".into());
    rows.replace(entries).unwrap();
    navigation.observe(1.into(), rows.rows(), &[4, 2, 2]);
    assert_eq!(navigation.anchors, [1, 2, 6]);
    assert_eq!(navigation.previous(top(5, 0.)), Some(2));
}

#[test]
fn truncate_chat_switch_clear_empty_and_tools_only_reset_targets() {
    let mut navigation = MessageNavigation::default();
    navigation.observe(1.into(), ManagedRows::new(mixed()).unwrap().rows(), &[]);
    navigation.observe(
        1.into(),
        ManagedRows::new(vec![message("short replacement")])
            .unwrap()
            .rows(),
        &[],
    );
    assert_eq!(navigation.anchors, [0]);
    navigation.observe(
        2.into(),
        ManagedRows::new(vec![Entry::Thought("another chat".into())])
            .unwrap()
            .rows(),
        &[],
    );
    assert!(
        navigation.anchors.is_empty(),
        "equal-length chat switch reused anchors"
    );
    navigation.observe(2.into(), &[], &[]);
    assert_eq!(navigation.previous(top(0, 10.)), None);
    assert_eq!(navigation.first(top(0, 10.)), None);
    navigation.observe(
        2.into(),
        ManagedRows::new(vec![Entry::Tool(tool("only", "Tool"))])
            .unwrap()
            .rows(),
        &[],
    );
    assert!(navigation.anchors.is_empty());
    assert_eq!(
        navigation.first(top(0, 10.)),
        Some(0),
        "chat top remains reachable without user rows"
    );
    navigation.clear();
    assert!(navigation.owner.is_none());
    assert_eq!(navigation.known_rows, 0);
}
