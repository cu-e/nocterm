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

#[test]
fn every_listed_hub_chat_reads_as_a_saved_chat_of_its_hub_agent() {
    use nocterm_ai::local_environment::ChatScope;
    let mut list = ChatList::default();
    list.replace(vec![row("h1", "one"), row("h2", "")]);
    list.remember("local-1", "h1");
    let scope = ChatScope::this_computer(nocterm_ai::history::ContextId::General);
    let chats: Vec<_> = list
        .local_ids()
        .filter_map(|id| super::saved_chat(&list, &scope, &hub(), &id))
        .collect();
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[0].header.title.as_deref(), Some("one"));
    assert_eq!(chats[0].session_id.as_deref(), Some("h1"));
    assert!(chats[1].header.title.is_none());
    assert!(
        chats[0]
            .header
            .agent_id
            .starts_with(nocterm_ai::agents::HUB_AGENT_PREFIX)
    );
}
