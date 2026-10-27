use super::*;
use crate::address::HubId;

fn hub(address: &str) -> Environment {
    Environment::Hub(HubId::from(HubAddress::parse(address).unwrap()))
}

fn environments(windows: &OpenWindows<u32>) -> Vec<Environment> {
    windows.document().environments().cloned().collect()
}

#[test]
fn a_document_round_trips_through_its_text() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.opened(2, hub("hub.home"));
    windows.opened(3, hub("[::1]:9000"));

    let text = windows.document().encode().unwrap();
    let read = WindowDocument::decode(&text).unwrap();

    assert_eq!(read, windows.document());
    assert_eq!(
        read.environments().cloned().collect::<Vec<_>>(),
        [Environment::Local, hub("hub.home:7443"), hub("[::1]:9000")]
    );
}

#[test]
fn the_text_names_each_environment_by_kind_and_canonical_address() {
    let mut windows = OpenWindows::default();
    windows.opened(1, hub("Hub.Home"));
    let value: serde_json::Value =
        serde_json::from_str(&windows.document().encode().unwrap()).unwrap();

    assert_eq!(value["version"], 1);
    assert_eq!(value["windows"][0]["environment"]["kind"], "hub");
    assert_eq!(value["windows"][0]["environment"]["hub"], "hub.home:7443");
}

#[test]
fn a_record_with_fields_this_version_does_not_know_is_still_read() {
    let text = r#"{"version":1,"windows":[
        {"environment":{"kind":"hub","hub":"hub.home:7443","sessions":["s1"]},"organization":"lab"},
        {"environment":{"kind":"local"}}]}"#;

    let read = WindowDocument::decode(text).unwrap();

    assert_eq!(
        read.environments().cloned().collect::<Vec<_>>(),
        [hub("hub.home:7443"), Environment::Local]
    );
}

#[test]
fn a_document_without_windows_is_empty() {
    assert_eq!(
        WindowDocument::decode("{}").unwrap(),
        WindowDocument::default()
    );
}

#[test]
fn text_that_is_not_a_document_is_unreadable() {
    for text in ["", "not json", r#"{"windows":"none"}"#] {
        assert!(
            matches!(
                WindowDocument::decode(text),
                Err(WindowDocumentError::Unreadable(_))
            ),
            "{text:?}"
        );
    }
}

#[test]
fn a_record_that_cannot_be_read_costs_only_itself() {
    let text = r#"{"version":1,"windows":[
        {"environment":{"kind":"moon"}},
        {"environment":{"kind":"hub","hub":"hub.home:7443"}},
        {"environment":{"kind":"hub","hub":"http://plain"}},
        {"organization":"lab"},
        {"environment":{"kind":"local"}}]}"#;

    let read = WindowDocument::decode(text).unwrap();

    assert_eq!(
        read.environments().cloned().collect::<Vec<_>>(),
        [hub("hub.home:7443"), Environment::Local]
    );
    assert_eq!(read.skipped(), 3);
}

#[test]
fn a_document_whose_records_all_read_skips_nothing() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);

    let read = WindowDocument::decode(&windows.document().encode().unwrap()).unwrap();

    assert_eq!(read.skipped(), 0);
}

#[test]
fn a_newer_version_is_refused_instead_of_misread() {
    assert!(matches!(
        WindowDocument::decode(r#"{"version":2,"windows":[]}"#),
        Err(WindowDocumentError::UnsupportedVersion {
            found: 2,
            supported: 1
        })
    ));
}

#[test]
fn the_unreadable_error_keeps_its_cause() {
    let error = WindowDocument::decode("nope").unwrap_err();
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn windows_are_listed_in_the_order_they_opened() {
    let mut windows = OpenWindows::default();
    windows.opened(7, hub("b.example"));
    windows.opened(3, Environment::Local);
    windows.opened(9, hub("a.example"));

    assert_eq!(
        environments(&windows),
        [hub("b.example"), Environment::Local, hub("a.example")]
    );
}

#[test]
fn a_closed_window_is_forgotten() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.opened(2, hub("hub.home"));
    windows.opened(3, Environment::Local);

    windows.closed(2);

    assert_eq!(
        environments(&windows),
        [Environment::Local, Environment::Local]
    );
}

#[test]
fn the_last_window_stays_when_it_closes() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.opened(2, hub("hub.home"));

    windows.closed(1);
    windows.closed(2);

    assert_eq!(environments(&windows), [hub("hub.home")]);
    assert_eq!(windows.len(), 1);
}

#[test]
fn closing_an_unknown_window_changes_nothing() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.opened(2, Environment::Local);

    windows.closed(5);

    assert_eq!(windows.len(), 2);
}

#[test]
fn switching_keeps_the_place_of_the_window() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.opened(2, Environment::Local);

    windows.switched(1, hub("hub.home"));
    windows.switched(8, hub("never.opened"));

    assert_eq!(
        environments(&windows),
        [hub("hub.home"), Environment::Local]
    );
}

#[test]
fn opening_a_key_twice_replaces_instead_of_duplicating() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.opened(1, hub("hub.home"));

    assert_eq!(environments(&windows), [hub("hub.home")]);
}

#[test]
fn a_kept_environment_stays_in_the_document_at_its_place() {
    let mut windows = OpenWindows::default();
    windows.kept(hub("gone.example"));
    windows.opened(1, Environment::Local);
    windows.kept(hub("later.example"));
    windows.opened(2, hub("hub.home"));

    assert_eq!(
        environments(&windows),
        [
            hub("gone.example"),
            Environment::Local,
            hub("later.example"),
            hub("hub.home")
        ]
    );
}

#[test]
fn a_kept_environment_is_not_a_window() {
    let mut windows = OpenWindows::default();
    windows.kept(hub("gone.example"));
    windows.opened(1, Environment::Local);

    windows.closed(1);

    assert_eq!(windows.len(), 1);
    assert_eq!(
        environments(&windows),
        [hub("gone.example"), Environment::Local]
    );
}

#[test]
fn closing_a_window_leaves_the_kept_environments() {
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.kept(hub("gone.example"));
    windows.opened(2, Environment::Local);

    windows.closed(2);

    assert_eq!(
        environments(&windows),
        [Environment::Local, hub("gone.example")]
    );
}

fn memory(organization: Option<&str>, sessions: &[&str]) -> HubWindowMemory {
    HubWindowMemory::new(
        organization.map(OrgId::new),
        sessions
            .iter()
            .map(|entry| {
                let (org, id) = entry.split_once('/').unwrap_or(("lab", entry));
                RememberedSession::new(OrgId::new(org), HubSessionId::new(id))
            })
            .collect(),
    )
}

fn hub_id(address: &str) -> HubId {
    HubId::from(HubAddress::parse(address).unwrap())
}

#[test]
fn the_organization_and_sessions_of_a_hub_window_round_trip_in_order() {
    let mut windows = OpenWindows::default();
    windows.opened(1, hub("hub.home"));
    windows.opened(2, Environment::Local);
    windows.remembered(
        1,
        &hub_id("hub.home"),
        memory(Some("lab"), &["s2", "s1", "s3"]),
    );

    let read = WindowDocument::decode(&windows.document().encode().unwrap()).unwrap();

    assert_eq!(read, windows.document());
    assert_eq!(
        read.records()[0].hub(),
        &memory(Some("lab"), &["s2", "s1", "s3"])
    );
    assert!(read.records()[1].hub().is_empty());
}

#[test]
fn tabs_of_several_organizations_are_remembered_each_with_its_own() {
    let mut windows = OpenWindows::default();
    windows.opened(1, hub("hub.home"));
    let shown = memory(Some("zoo"), &["lab/s1", "zoo/z1"]);
    windows.remembered(1, &hub_id("hub.home"), shown.clone());

    let read = WindowDocument::decode(&windows.document().encode().unwrap()).unwrap();

    assert_eq!(read.records()[0].hub(), &shown);
}

#[test]
fn the_text_keeps_the_memory_under_the_hub_key_of_the_record() {
    let mut windows = OpenWindows::default();
    windows.opened(1, hub("hub.home"));
    windows.remembered(1, &hub_id("hub.home"), memory(Some("lab"), &["s1"]));
    let value: serde_json::Value =
        serde_json::from_str(&windows.document().encode().unwrap()).unwrap();

    assert_eq!(value["version"], 1);
    assert_eq!(value["windows"][0]["hub"]["organization"], "lab");
    assert_eq!(value["windows"][0]["hub"]["sessions"][0]["id"], "s1");
    assert_eq!(value["windows"][0]["hub"]["sessions"][0]["org"], "lab");
}

#[test]
fn a_window_that_kept_nothing_is_written_as_before() {
    let mut windows = OpenWindows::default();
    windows.opened(1, hub("hub.home"));
    windows.opened(2, Environment::Local);
    let value: serde_json::Value =
        serde_json::from_str(&windows.document().encode().unwrap()).unwrap();

    assert!(value["windows"][0].get("hub").is_none());
    assert!(value["windows"][1].get("hub").is_none());
}

#[test]
fn a_document_of_the_first_format_loads_with_empty_memory() {
    let text = r#"{"version":1,"windows":[
        {"environment":{"kind":"hub","hub":"hub.home:7443"}},
        {"environment":{"kind":"local"}}]}"#;

    let read = WindowDocument::decode(text).unwrap();

    assert_eq!(read.records().len(), 2);
    assert!(read.records().iter().all(|record| record.hub().is_empty()));
}

#[test]
fn memory_that_cannot_be_read_costs_the_memory_not_the_window() {
    let text = r#"{"version":1,"windows":[
        {"environment":{"kind":"hub","hub":"hub.home:7443"},"hub":"nonsense"},
        {"environment":{"kind":"hub","hub":"hub.home:7443"},"hub":{"organization":"lab","sessions":"s1"}},
        {"environment":{"kind":"hub","hub":"hub.home:7443"},"hub":{"organization":"lab","future":1}}]}"#;

    let read = WindowDocument::decode(text).unwrap();

    assert_eq!(read.skipped(), 0);
    assert_eq!(read.records().len(), 3);
    assert!(read.records()[0].hub().is_empty());
    assert!(read.records()[1].hub().is_empty());
    assert_eq!(read.records()[2].hub(), &memory(Some("lab"), &[]));
}

#[test]
fn memory_is_given_only_to_an_open_window_that_still_shows_that_hub() {
    let mut windows = OpenWindows::default();
    windows.opened(1, hub("a.example"));
    windows.opened(2, Environment::Local);

    // A report from before the window was switched to another server.
    windows.switched(1, hub("b.example"));
    windows.remembered(1, &hub_id("a.example"), memory(None, &["s1"]));
    // A window of this computer and a window that is not open keep nothing.
    windows.remembered(2, &hub_id("a.example"), memory(None, &["s1"]));
    windows.remembered(9, &hub_id("b.example"), memory(None, &["s1"]));

    assert!(
        windows
            .document()
            .records()
            .iter()
            .all(|record| record.hub().is_empty())
    );
}

#[test]
fn switching_a_window_forgets_what_it_kept_of_the_previous_hub() {
    let mut windows = OpenWindows::default();
    windows.opened(1, hub("a.example"));
    windows.remembered(1, &hub_id("a.example"), memory(Some("lab"), &["s1"]));

    windows.switched(1, hub("b.example"));

    assert!(windows.document().records()[0].hub().is_empty());
}

#[test]
fn a_restored_window_comes_back_with_its_memory_and_a_local_one_with_none() {
    let mut windows = OpenWindows::default();
    windows.restored(
        1,
        WindowRecord::new(hub("hub.home")).with_hub(memory(Some("lab"), &["s1"])),
    );
    windows.restored(
        2,
        WindowRecord::new(Environment::Local).with_hub(memory(Some("lab"), &["s1"])),
    );

    let records = windows.document();
    assert_eq!(records.records()[0].hub(), &memory(Some("lab"), &["s1"]));
    assert!(records.records()[1].hub().is_empty());
}

#[test]
fn a_kept_record_keeps_its_memory() {
    let mut windows = OpenWindows::default();
    windows.kept(WindowRecord::new(hub("gone.example")).with_hub(memory(None, &["s1"])));
    windows.opened(1, Environment::Local);

    assert_eq!(
        windows.document().records()[0].hub(),
        &memory(None, &["s1"])
    );
}
