use gpui::TestAppContext;
use nocterm_plugin_api::{
    Effect, HostError,
    capability::storage::{MAX_TOTAL, MAX_VALUE, Remove, Set},
};
use nocterm_plugin_model::{SettingValue, SettingValues};

use super::rig::{Rig, effects};
use crate::{
    DataBoundsError, MAX_VALUES_BYTES, PluginDataChange, PluginDataDoc, PluginDataError,
    RecordsDoc, StoredValue,
};

fn set(key: &str, value: &str) -> Set {
    Set {
        key: key.into(),
        value: value.into(),
    }
}

#[test]
fn storage_is_bounded_per_value_and_in_total() {
    let doc = PluginDataDoc::default();
    assert!(doc.with_set(&set("k", &"x".repeat(MAX_VALUE))).is_ok());
    assert!(matches!(
        doc.with_set(&set("k", &"x".repeat(MAX_VALUE + 1))),
        Err(DataBoundsError::Entry(_))
    ));
    assert!(matches!(
        doc.with_set(&set("Bad Key", "v")),
        Err(DataBoundsError::Entry(_))
    ));

    let mut full = doc;
    for index in 0..MAX_TOTAL / MAX_VALUE {
        full = full
            .with_set(&set(&format!("k{index:02}"), &"x".repeat(MAX_VALUE - 3)))
            .expect("inside the total");
    }
    assert!(matches!(
        full.with_set(&set("more", &"x".repeat(MAX_VALUE))),
        Err(DataBoundsError::StorageTooLarge { max: MAX_TOTAL })
    ));
    let freed = full
        .with_removed(&Remove { key: "k00".into() })
        .expect("a remove always fits");
    assert!(freed.with_set(&set("more", "v")).is_ok());
}

#[test]
fn setting_values_are_bounded_when_serialized() {
    let mut values = SettingValues::default();
    values.insert("city", SettingValue::Text("x".repeat(MAX_VALUES_BYTES)));
    assert!(matches!(
        PluginDataDoc::default().with_values(&values),
        Err(DataBoundsError::ValuesTooLarge { .. })
    ));
    values.insert("city", SettingValue::Text("Oslo".into()));
    let doc = PluginDataDoc::default().with_values(&values).unwrap();
    assert_eq!(doc.setting_values(), values);
}

#[test]
fn a_document_round_trips_through_json_and_refuses_unknown_keys() {
    let doc = PluginDataDoc {
        values: [
            ("a".to_owned(), StoredValue::Bool(true)),
            ("b".to_owned(), StoredValue::Choices(vec!["x".into()])),
            ("c".to_owned(), StoredValue::Number(-3)),
        ]
        .into(),
        storage: [("k".to_owned(), "v".to_owned())].into(),
    };
    assert_eq!(PluginDataDoc::decode(&doc.encode().unwrap()).unwrap(), doc);
    assert!(PluginDataDoc::decode(br#"{"extra":1}"#).is_err());
    assert_eq!(
        PluginDataDoc::decode(b"{}").unwrap(),
        PluginDataDoc::default()
    );
}

#[test]
fn decoding_refuses_a_document_over_its_bounds_and_trims_the_records() {
    let big = "x".repeat(MAX_VALUES_BYTES);
    let text = format!(r#"{{"values":{{"a":{{"kind":"text","value":"{big}"}}}}}}"#);
    assert!(PluginDataDoc::decode(text.as_bytes()).is_err());

    let audit: Vec<_> = (0..200)
        .map(|seq| format!(r#"{{"seq":{seq},"plugin":"user-a-b","action":"enabled"}}"#))
        .collect();
    let text = format!(r#"{{"audit":[{}]}}"#, audit.join(","));
    let doc = RecordsDoc::decode(text.as_bytes()).unwrap();
    assert_eq!(doc.audit.len(), crate::records::MAX_AUDIT);
    assert_eq!(doc.audit.last().map(|entry| entry.seq), Some(199));
    assert_eq!(RecordsDoc::decode(&doc.encode().unwrap()).unwrap(), doc);
    assert!(RecordsDoc::decode(b"not json").is_err());
}

#[test]
fn a_record_whose_id_is_not_a_user_plugin_id_is_dropped_on_read() {
    let text = r#"{"plugins":[
        {"id":"files","hash":"00","enabled":true},
        {"id":"user-acme-clock","hash":"00","enabled":true},
        {"id":"user-acme-clock","hash":"11","enabled":false}
    ]}"#;
    let doc = RecordsDoc::decode(text.as_bytes()).unwrap();
    let ids: Vec<_> = doc
        .plugins
        .iter()
        .map(|record| record.id.as_str())
        .collect();
    assert_eq!(ids, ["user-acme-clock"]);
    assert_eq!(doc.plugins[0].hash, "00", "the first record of an id wins");
}

#[gpui::test]
fn the_storage_effects_write_the_plugins_own_document_through_the_port(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.script.answer_event(Ok(effects(vec![
        (Some("a"), Effect::StorageSet(set("count", "1"))),
        (Some("b"), Effect::StorageSet(set("name", "x"))),
        (
            Some("c"),
            Effect::StorageRemove(Remove { key: "name".into() }),
        ),
    ])));
    rig.press("face", cx).unwrap();

    let stored = rig.data.stored(&id).unwrap();
    assert_eq!(stored.storage.get("count").map(String::as_str), Some("1"));
    assert!(!stored.storage.contains_key("name"));
    assert_eq!(rig.data.changes().len(), 3);

    rig.render("face", cx).unwrap();
    let snapshot = rig.script.snapshots().pop().unwrap();
    assert_eq!(snapshot.storage.get("count").map(String::as_str), Some("1"));
}

#[gpui::test]
fn a_set_past_the_total_is_denied_and_changes_nothing(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    let mut doc = PluginDataDoc::default();
    for index in 0..MAX_TOTAL / MAX_VALUE {
        doc = doc
            .with_set(&set(&format!("k{index:02}"), &"x".repeat(MAX_VALUE - 3)))
            .unwrap();
    }
    rig.data.put(&id, doc.clone());
    rig.restore(cx);
    rig.script.answer_event(Ok(effects(vec![(
        Some("a"),
        Effect::StorageSet(set("more", &"y".repeat(MAX_VALUE))),
    )])));
    rig.press("face", cx).unwrap();

    assert_eq!(rig.data.stored(&id), Some(doc));
    assert_eq!(
        rig.outcomes(cx),
        [(
            Some("a".to_owned()),
            Err(nocterm_plugin_api::HostError::Denied)
        )]
    );
}

#[gpui::test]
fn a_broken_file_gives_defaults_stays_untouched_and_is_replaced_by_the_first_edit(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.break_file(&id);
    rig.restore(cx);

    let plugin = &rig.host.read_with(cx, |host, _| host.plugins())[0];
    assert!(plugin.data_issue.is_some(), "the list says why");
    rig.render("face", cx).unwrap();
    let snapshot = rig.script.snapshots().pop().unwrap();
    assert_eq!(
        snapshot.settings.get("city"),
        Some(&SettingValue::Text(String::new())),
        "defaults"
    );
    assert!(snapshot.storage.is_empty());
    assert!(rig.data.is_broken(&id), "reading never rewrites the file");
    assert!(rig.data.changes().is_empty());

    let mut values = SettingValues::default();
    values.insert("city", SettingValue::Text("Oslo".into()));
    rig.host
        .update(cx, |host, cx| host.replace_values(&id, &values, cx))
        .unwrap();
    assert!(!rig.data.is_broken(&id), "the edit replaced it");
    assert_eq!(rig.data.stored(&id).unwrap().setting_values(), values);
    assert_eq!(rig.data.changes().len(), 1);
    assert!(matches!(
        rig.data.changes()[0].1,
        PluginDataChange::Replace(_)
    ));
}

#[gpui::test]
fn a_storage_effect_never_overwrites_a_file_that_could_not_be_read(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.break_file(&id);
    rig.restore(cx);
    rig.script.answer_event(Ok(effects(vec![
        (Some("a"), Effect::StorageSet(set("count", "1"))),
        (Some("b"), Effect::StorageRemove(Remove { key: "x".into() })),
    ])));
    rig.press("face", cx).unwrap();

    assert!(rig.data.changes().is_empty(), "the port was never asked");
    assert!(rig.data.is_broken(&id), "the file is as it was");
    assert_eq!(
        rig.outcomes(cx),
        [
            (Some("a".to_owned()), Err(HostError::Unavailable)),
            (Some("b".to_owned()), Err(HostError::Unavailable)),
        ]
    );
}

#[gpui::test]
fn storage_works_again_once_the_user_replaced_a_broken_file(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.break_file(&id);
    rig.restore(cx);
    rig.host
        .update(cx, |host, cx| {
            host.replace_values(&id, &SettingValues::default(), cx)
        })
        .unwrap();
    rig.script.answer_event(Ok(effects(vec![(
        Some("a"),
        Effect::StorageSet(set("count", "1")),
    )])));
    rig.press("face", cx).unwrap();

    assert_eq!(rig.outcomes(cx), [(Some("a".to_owned()), Ok(()))]);
    let stored = rig.data.stored(&id).unwrap();
    assert_eq!(stored.storage.get("count").map(String::as_str), Some("1"));
}

#[gpui::test]
fn why_a_file_is_broken_keeps_the_original_cause(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.break_file(&id);
    rig.restore(cx);
    let plugin = &rig.host.read_with(cx, |host, _| host.plugins())[0];
    let issue = plugin.data_issue.as_ref().expect("the list says why");

    assert!(matches!(issue, PluginDataError::Broken(_)));
    let mut chain = Vec::new();
    let mut source = std::error::Error::source(issue);
    while let Some(error) = source {
        chain.push(error.to_string());
        source = error.source();
    }
    assert!(
        chain
            .iter()
            .any(|link| link.contains("not a valid plugin document")),
        "{chain:?}"
    );
}

#[gpui::test]
fn a_file_over_its_bounds_counts_as_broken(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.put(
        &id,
        PluginDataDoc {
            values: [(
                "city".to_owned(),
                StoredValue::Text("x".repeat(MAX_VALUES_BYTES)),
            )]
            .into(),
            ..PluginDataDoc::default()
        },
    );
    rig.restore(cx);
    let plugin = &rig.host.read_with(cx, |host, _| host.plugins())[0];
    assert!(matches!(
        plugin.data_issue,
        Some(PluginDataError::Bounds(
            DataBoundsError::ValuesTooLarge { .. }
        ))
    ));
}

#[gpui::test]
fn edited_values_over_their_bound_are_refused(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    let mut values = SettingValues::default();
    values.insert("city", SettingValue::Text("x".repeat(MAX_VALUES_BYTES)));
    let refused = rig
        .host
        .update(cx, |host, cx| host.replace_values(&id, &values, cx));
    assert!(matches!(refused, Err(crate::ManageError::Data(_))));
    assert!(rig.data.changes().is_empty());
}

#[gpui::test]
fn a_storage_effect_the_port_refuses_is_not_kept_in_memory_either(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.refuse_changes();
    rig.script.answer_event(Ok(effects(vec![(
        Some("a"),
        Effect::StorageSet(set("count", "1")),
    )])));
    rig.press("face", cx).unwrap();

    assert_eq!(
        rig.outcomes(cx),
        [(Some("a".to_owned()), Err(HostError::Unavailable))]
    );
    rig.render("face", cx).unwrap();
    let snapshot = rig.script.snapshots().pop().unwrap();
    assert!(
        snapshot.storage.is_empty(),
        "the host does not believe it stored what the port refused"
    );
    assert!(rig.data.stored(&id).is_none());
}

#[gpui::test]
fn edited_values_the_port_refuses_are_reported_and_not_kept(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.refuse_changes();
    let mut values = SettingValues::default();
    values.insert("city", SettingValue::Text("Oslo".into()));
    let refused = rig
        .host
        .update(cx, |host, cx| host.replace_values(&id, &values, cx));

    assert!(matches!(
        refused,
        Err(crate::ManageError::Unsaved(PluginDataError::NotSaved(_)))
    ));
    rig.render("face", cx).unwrap();
    let snapshot = rig.script.snapshots().pop().unwrap();
    assert_ne!(
        snapshot.settings.get("city"),
        Some(&SettingValue::Text("Oslo".into()))
    );
}
