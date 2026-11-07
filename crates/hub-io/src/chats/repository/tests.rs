use nocterm_hub_proto::v2;
use nocterm_hubs_model::{HubAddress, HubId};

use super::list::{ChatList, Row};

fn hub() -> HubId {
    HubId::from(HubAddress::parse("hub.example:7443").unwrap())
}

fn row(id: &str, title: &str) -> Row {
    Row::of(&v2::Chat {
        id: id.into(),
        agent_id: "a1".into(),
        agent_name: "Ops".into(),
        title: title.into(),
        updated_at_ms: 5_000,
        ..Default::default()
    })
}

#[test]
fn a_chat_started_here_keeps_its_own_id_in_the_history() {
    let mut list = ChatList::default();
    assert!(list.replace(vec![row("h1", "one"), row("h2", "two")]));
    list.remember("local-1", "h1");
    let ids: Vec<_> = list.local_ids().collect();
    assert_eq!(ids, ["local-1", "h2"]);
    assert_eq!(list.row_for("local-1").unwrap().id, "h1");
    assert_eq!(list.row_for("h2").unwrap().updated(), 5);
    assert!(!list.replace(vec![row("h1", "one"), row("h2", "two")]));
}

#[test]
fn a_chat_the_hub_no_longer_lists_is_forgotten() {
    let mut list = ChatList::default();
    list.replace(vec![row("h1", "one")]);
    list.remember("local-1", "h1");
    list.replace(vec![row("h2", "two")]);
    assert!(list.row_for("local-1").is_none());
    assert_eq!(list.forget("h2").as_deref(), Some("h2"));
    assert_eq!(list.local_ids().count(), 0);
}

#[test]
fn a_rename_shows_before_the_hub_confirms_it_and_the_list_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut list = ChatList::default();
    list.replace(vec![row("h1", "one")]);
    list.apply("h1", Some("renamed"), Some(true));
    list.save(dir.path(), &hub());
    let kept = ChatList::load(dir.path(), &hub());
    let shown = kept.row_for("h1").unwrap();
    assert_eq!(shown.title, "renamed");
    assert!(shown.pinned);
    assert_eq!(
        ChatList::load(
            dir.path(),
            &HubId::from(HubAddress::parse("other:1").unwrap())
        ),
        ChatList::default()
    );
}
