use gpui::TestAppContext;
use nocterm_plugin_api::{ContentHash, HostError};

use super::rig::{Rig, activated, effects, post_effect};
use crate::{
    AuditAction, BrokenPackage, CallError, Input, MAX_ENABLED, ManageError, PluginDataChange,
    PluginDataDoc, PluginRecord, PluginStatus, RecordsDoc, StoredValue,
    test_support::{MemoryData, MemoryPackages, MemoryRecords, package, package_named, plugin_id},
};

fn record(hash_byte: u8, enabled: bool) -> PluginRecord {
    PluginRecord {
        id: plugin_id(),
        hash: ContentHash::new([hash_byte; 32]).to_string(),
        enabled,
        placeable: true,
        granted_optional: Vec::new(),
    }
}

#[gpui::test]
fn installing_a_package_stores_it_records_the_consent_and_lists_the_plugin(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    let id = rig.install(package(1), &["monitor.usage"], cx).unwrap();

    assert!(rig.packages.holds(&id));
    let plugins = rig.host.read_with(cx, |host, _| host.plugins());
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].status, PluginStatus::Running);
    assert!(plugins[0].enabled);
    assert!(
        !plugins[0].placeable,
        "placement needs the user's separate yes"
    );
    let doc = rig.records.doc();
    assert_eq!(doc.plugins[0].granted_optional, ["monitor.usage"]);
    assert_eq!(doc.plugins[0].hash, ContentHash::new([1; 32]).to_string());
    assert_eq!(doc.audit.len(), 1);
    assert_eq!(doc.audit[0].action, AuditAction::Installed);
}

#[gpui::test]
fn an_optional_capability_that_the_package_does_not_list_is_refused_at_consent(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    let error = rig.install(package(1), &["clock.now"], cx).unwrap_err();
    assert!(matches!(error, ManageError::Grant(_)));
    assert!(!rig.packages.holds(&plugin_id()), "nothing was stored");
    assert_eq!(rig.records.saves(), 0);
}

#[gpui::test]
fn a_plugin_id_is_installed_once(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    assert_eq!(
        rig.install(package(2), &[], cx),
        Err(ManageError::Installed(plugin_id()))
    );
}

#[gpui::test]
fn a_new_run_loads_the_plugins_whose_package_still_has_the_accepted_hash(cx: &mut TestAppContext) {
    let packages = MemoryPackages::new();
    packages.put(&package(7));
    let records = MemoryRecords::seeded(RecordsDoc {
        plugins: vec![record(7, true)],
        audit: Vec::new(),
    });
    let rig = Rig::over(records, MemoryData::new(), packages, cx);
    rig.restore(cx);

    let plugins = rig.host.read_with(cx, |host, _| host.plugins());
    assert_eq!(plugins[0].status, PluginStatus::Running);
    assert!(plugins[0].placeable, "placement for this hash is kept");
    assert!(rig.render("face", cx).is_ok());
}

#[gpui::test]
fn a_package_that_changed_on_disk_is_listed_broken_and_never_runs(cx: &mut TestAppContext) {
    let packages = MemoryPackages::new();
    packages.put(&package(8));
    let records = MemoryRecords::seeded(RecordsDoc {
        plugins: vec![record(7, true)],
        audit: Vec::new(),
    });
    let rig = Rig::over(records, MemoryData::new(), packages, cx);
    rig.restore(cx);

    let plugins = rig.host.read_with(cx, |host, _| host.plugins());
    assert!(matches!(
        plugins[0].status,
        PluginStatus::Broken(BrokenPackage::Package(_))
    ));
    assert_eq!(
        rig.render("face", cx),
        Err(CallError::NotRunning(plugin_id()))
    );
    assert_eq!(rig.script.instances(), 0);
}

#[gpui::test]
fn a_record_with_a_missing_package_or_a_bad_hash_is_broken(cx: &mut TestAppContext) {
    let mut garbled = record(1, true);
    garbled.hash = "not a hash".into();
    let records = MemoryRecords::seeded(RecordsDoc {
        plugins: vec![garbled],
        audit: Vec::new(),
    });
    let rig = Rig::over(records, MemoryData::new(), MemoryPackages::new(), cx);
    rig.restore(cx);
    let plugins = rig.host.read_with(cx, |host, _| host.plugins());
    assert!(matches!(
        plugins[0].status,
        PluginStatus::Broken(BrokenPackage::Hash(_))
    ));

    let records = MemoryRecords::seeded(RecordsDoc {
        plugins: vec![record(1, true)],
        audit: Vec::new(),
    });
    let rig = Rig::over(records, MemoryData::new(), MemoryPackages::new(), cx);
    rig.restore(cx);
    let plugins = rig.host.read_with(cx, |host, _| host.plugins());
    assert!(matches!(
        plugins[0].status,
        PluginStatus::Broken(BrokenPackage::Package(_))
    ));
}

#[gpui::test]
fn the_records_document_is_bounded_on_load(cx: &mut TestAppContext) {
    let packages = MemoryPackages::new();
    packages.put(&package(7));
    let records = MemoryRecords::seeded(RecordsDoc {
        plugins: vec![record(7, true), record(7, false)],
        audit: Vec::new(),
    });
    let rig = Rig::over(records, MemoryData::new(), packages, cx);
    rig.restore(cx);
    assert_eq!(rig.host.read_with(cx, |host, _| host.plugins()).len(), 1);
}

#[gpui::test]
fn at_most_sixteen_plugins_are_enabled(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    for number in 0..MAX_ENABLED {
        let name = format!("p{number}");
        rig.install(package_named(&name, number as u8), &[], cx)
            .unwrap();
    }
    let refused = rig.install(package_named("extra", 99), &[], cx);
    assert_eq!(
        refused,
        Err(ManageError::TooManyEnabled { max: MAX_ENABLED })
    );

    let first = nocterm_plugin_model::PluginId::owned("user-acme-p0");
    rig.host
        .update(cx, |host, cx| host.set_enabled(&first, false, cx))
        .unwrap();
    rig.install(package_named("extra", 99), &[], cx).unwrap();
    let again = rig
        .host
        .update(cx, |host, cx| host.set_enabled(&first, true, cx));
    assert_eq!(again, Err(ManageError::TooManyEnabled { max: MAX_ENABLED }));
}

#[gpui::test]
fn a_restore_enables_no_more_than_sixteen(cx: &mut TestAppContext) {
    let packages = MemoryPackages::new();
    let mut plugins = Vec::new();
    for number in 0..MAX_ENABLED + 2 {
        let package = package_named(&format!("p{number}"), number as u8 + 1);
        packages.put(&package);
        plugins.push(PluginRecord {
            id: package.manifest().plugin().id().clone(),
            hash: package.hash().to_string(),
            enabled: true,
            placeable: false,
            granted_optional: Vec::new(),
        });
    }
    let records = MemoryRecords::seeded(RecordsDoc {
        plugins,
        audit: Vec::new(),
    });
    let rig = Rig::over(records, MemoryData::new(), packages, cx);
    rig.restore(cx);

    let summaries = rig.host.read_with(cx, |host, _| host.plugins());
    assert_eq!(
        summaries.iter().filter(|plugin| plugin.enabled).count(),
        MAX_ENABLED
    );
    assert_eq!(
        rig.records
            .doc()
            .plugins
            .iter()
            .filter(|r| r.enabled)
            .count(),
        MAX_ENABLED
    );
}

#[gpui::test]
fn a_disabled_plugin_is_not_instantiated_and_a_disable_is_audited(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.render("face", cx).unwrap();
    assert_eq!(rig.script.instances(), 1);

    rig.host
        .update(cx, |host, cx| host.set_enabled(&id, false, cx))
        .unwrap();
    assert_eq!(
        rig.render("face", cx),
        Err(CallError::NotRunning(id.clone()))
    );
    rig.host
        .update(cx, |host, cx| host.set_enabled(&id, true, cx))
        .unwrap();
    rig.render("face", cx).unwrap();
    assert_eq!(rig.script.instances(), 2, "a new instance after the switch");
    let actions: Vec<_> = rig.records.doc().audit.iter().map(|e| e.action).collect();
    assert_eq!(
        actions,
        [
            AuditAction::Installed,
            AuditAction::Disabled,
            AuditAction::Enabled
        ]
    );
}

#[gpui::test]
fn disabling_drops_every_effect_still_queued(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("a"), post_effect("hello"))])));
    let call = rig.call("face");
    rig.host.update(cx, |host, cx| {
        host.handle(&call, Input::Pointer, activated(), &rig.providers, cx)
            .unwrap();
        // Still in the same update: the effect waits for the deferred apply. Switching the
        // plugin off and on again must not bring it back.
        host.set_enabled(&id, false, cx).unwrap();
        host.set_enabled(&id, true, cx).unwrap();
    });
    assert!(rig.post.requests().is_empty());
}

#[gpui::test]
fn replacing_drops_queued_effects_and_their_outcomes(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("a"), post_effect("one"))])));
    let call = rig.call("face");
    rig.host.update(cx, |host, cx| {
        host.handle(&call, Input::Pointer, activated(), &rig.providers, cx)
            .unwrap();
        host.replace(package(2), &[], false, cx).unwrap();
    });
    assert!(rig.post.requests().is_empty());
    assert!(
        rig.outcomes(cx).is_empty(),
        "nothing is reported for a dropped effect"
    );
}

#[gpui::test]
fn uninstalling_drops_queued_effects_even_if_the_same_package_comes_back(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(None, post_effect("two"))])));
    let call = rig.call("face");
    rig.host.update(cx, |host, cx| {
        host.handle(&call, Input::Pointer, activated(), &rig.providers, cx)
            .unwrap();
        host.uninstall(&id, false, cx).unwrap();
        host.install(package(1), &[], cx).unwrap();
    });
    assert!(rig.post.requests().is_empty());
}

#[gpui::test]
fn a_replacement_starts_not_placeable_and_keeps_data_only_when_asked(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.host
        .update(cx, |host, cx| host.set_placeable(&id, true, cx))
        .unwrap();
    let doc = PluginDataDoc {
        storage: [("k".to_owned(), "v".to_owned())].into(),
        ..PluginDataDoc::default()
    };
    rig.data.put(&id, doc.clone());
    rig.restore(cx);

    rig.host
        .update(cx, |host, cx| host.replace(package(2), &[], true, cx))
        .unwrap();
    let plugin = &rig.host.read_with(cx, |host, _| host.plugins())[0];
    assert!(!plugin.placeable, "a new hash needs a new yes");
    assert_eq!(plugin.hash, ContentHash::new([2; 32]).to_string());
    assert_eq!(rig.data.stored(&id), Some(doc), "kept");

    rig.host
        .update(cx, |host, cx| host.replace(package(3), &[], false, cx))
        .unwrap();
    assert_eq!(rig.data.stored(&id), None, "removed");
    assert_eq!(
        rig.data.changes().last().map(|(_, change)| change.clone()),
        Some(PluginDataChange::Remove)
    );
}

#[gpui::test]
fn a_replacement_needs_an_installed_plugin_of_the_same_id(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    let other = rig.host.update(cx, |host, cx| {
        host.replace(package_named("other", 2), &[], true, cx)
    });
    assert!(matches!(other, Err(ManageError::Unknown(_))));
}

#[gpui::test]
fn removing_a_plugin_deletes_its_package_and_data_on_request(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.put(
        &id,
        PluginDataDoc {
            values: [("city".to_owned(), StoredValue::Text("Oslo".into()))].into(),
            ..PluginDataDoc::default()
        },
    );
    rig.host
        .update(cx, |host, cx| host.uninstall(&id, false, cx))
        .unwrap();
    assert!(!rig.packages.holds(&id));
    assert!(rig.data.stored(&id).is_some(), "data stays unless asked");
    assert!(rig.host.read_with(cx, |host, _| host.plugins()).is_empty());
    assert_eq!(rig.render("face", cx), Err(CallError::Unknown(id.clone())));

    rig.installed(cx);
    rig.host
        .update(cx, |host, cx| host.uninstall(&id, true, cx))
        .unwrap();
    assert_eq!(rig.data.stored(&id), None);
    assert_eq!(
        rig.host
            .update(cx, |host, cx| host.uninstall(&id, true, cx)),
        Err(ManageError::Unknown(id))
    );
}

#[gpui::test]
fn removing_the_data_alone_keeps_the_plugin_and_is_audited(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.put(&id, PluginDataDoc::default());

    rig.host
        .update(cx, |host, cx| host.remove_data(&id, cx))
        .unwrap();

    assert_eq!(rig.data.stored(&id), None);
    assert_eq!(rig.host.read_with(cx, |host, _| host.plugins()).len(), 1);
    let last = rig
        .host
        .read_with(cx, |host, _| host.audit().last().cloned());
    assert_eq!(
        last.map(|entry| entry.action),
        Some(AuditAction::DataRemoved)
    );
}

#[gpui::test]
fn removing_the_data_drops_the_effects_still_queued(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.script.answer_event(Ok(effects(vec![(
        Some("a"),
        nocterm_plugin_api::Effect::StorageSet(nocterm_plugin_api::capability::storage::Set {
            key: "count".into(),
            value: "1".into(),
        }),
    )])));
    let call = rig.call("face");
    rig.host.update(cx, |host, cx| {
        host.handle(&call, Input::Pointer, activated(), &rig.providers, cx)
            .unwrap();
        // Still in the same update: the write waits for the deferred apply.
        host.remove_data(&id, cx).unwrap();
    });
    cx.run_until_parked();

    let changes = rig.data.changes();

    assert!(
        changes
            .iter()
            .all(|(_, change)| matches!(change, PluginDataChange::Remove)),
        "no key came back after the removal: {changes:?}"
    );
}

#[gpui::test]
fn removing_the_data_is_refused_when_the_port_cannot_take_it(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.put(&id, PluginDataDoc::default());
    rig.data.refuse_changes();

    let refused = rig.host.update(cx, |host, cx| host.remove_data(&id, cx));

    assert!(matches!(refused, Err(ManageError::Unsaved(_))));
    assert!(rig.data.stored(&id).is_some());
    let last = rig
        .host
        .read_with(cx, |host, _| host.audit().last().cloned());
    assert_ne!(
        last.map(|entry| entry.action),
        Some(AuditAction::DataRemoved),
        "a removal that did not happen is not audited"
    );
}

#[test]
fn a_call_error_maps_to_the_error_a_plugin_may_see() {
    assert_eq!(
        HostError::from(CallError::Throttled { retry_after_ms: 5 }),
        HostError::RateLimited { retry_after_ms: 5 }
    );
    assert_eq!(
        HostError::from(CallError::NotRunning(plugin_id())),
        HostError::Unavailable
    );
}
