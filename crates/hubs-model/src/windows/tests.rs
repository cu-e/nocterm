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
