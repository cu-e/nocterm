//! The session-only ports keep their documents in memory and hand them back like the file ports.

use gpui::TestAppContext;
use nocterm_plugin_model::PluginId;

use crate::{
    PluginData, PluginDataChange, PluginDataDoc, PluginRecords, RecordsDoc, SessionData,
    SessionRecords,
    records::{AuditAction, AuditEntry},
    test_support::plugin_id,
};

fn doc_with(key: &str) -> PluginDataDoc {
    PluginDataDoc {
        storage: [(key.to_owned(), "v".to_owned())].into(),
        ..PluginDataDoc::default()
    }
}

#[gpui::test]
fn records_start_empty_and_hand_back_what_was_replaced(cx: &mut TestAppContext) {
    let records = SessionRecords::new();
    assert_eq!(cx.update(|cx| records.load(cx)), RecordsDoc::default());

    let doc = RecordsDoc {
        plugins: Vec::new(),
        audit: vec![AuditEntry {
            seq: 1,
            plugin: plugin_id(),
            action: AuditAction::Installed,
        }],
    };
    let saved = cx.update(|cx| records.replace(doc.clone(), cx));
    cx.foreground_executor().block_test(saved).unwrap();
    assert_eq!(cx.update(|cx| records.load(cx)), doc);
    assert_eq!(records.doc(), doc);
}

#[gpui::test]
fn records_can_start_from_a_seeded_document(cx: &mut TestAppContext) {
    let doc = RecordsDoc {
        plugins: Vec::new(),
        audit: vec![AuditEntry {
            seq: 7,
            plugin: plugin_id(),
            action: AuditAction::Enabled,
        }],
    };
    let records = SessionRecords::seeded(doc.clone());
    assert_eq!(cx.update(|cx| records.load(cx)), doc);
}

#[gpui::test]
fn data_of_an_unknown_plugin_is_the_default_document(cx: &mut TestAppContext) {
    let data = SessionData::new();
    assert_eq!(
        cx.update(|cx| data.load(&plugin_id(), cx)).unwrap(),
        PluginDataDoc::default()
    );
}

#[gpui::test]
fn data_changes_replace_and_remove_one_plugin_without_touching_another(cx: &mut TestAppContext) {
    let data = SessionData::new();
    let other = PluginId::owned("user-acme-other");
    cx.update(|cx| {
        data.change(plugin_id(), PluginDataChange::Replace(doc_with("a")), cx)
            .unwrap();
        data.change(other.clone(), PluginDataChange::Replace(doc_with("b")), cx)
            .unwrap();
    });
    assert_eq!(data.stored(&plugin_id()), Some(doc_with("a")));
    assert_eq!(
        cx.update(|cx| data.load(&other, cx)).unwrap(),
        doc_with("b")
    );

    cx.update(|cx| {
        data.change(plugin_id(), PluginDataChange::Remove, cx)
            .unwrap()
    });
    assert_eq!(data.stored(&plugin_id()), None);
    assert_eq!(data.stored(&other), Some(doc_with("b")));
}
