//! What the shell and the plugins page read from the host.

use gpui::TestAppContext;
use nocterm_plugin_api::PackageError;

use super::rig::Rig;
use crate::test_support::{package, plugin_id};

#[gpui::test]
fn the_entries_follow_enablement_and_placement(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.install(package(1), &[], cx).unwrap();

    let entries = rig.host.read_with(cx, |host, _| host.entries());
    assert_eq!(entries.len(), 1);
    assert!(entries[0].enabled);
    assert!(!entries[0].placeable, "a new package waits for approval");

    rig.host.update(cx, |host, cx| {
        host.set_placeable(&plugin_id(), true, cx).unwrap();
        host.set_enabled(&plugin_id(), false, cx).unwrap();
    });

    let entries = rig.host.read_with(cx, |host, _| host.entries());
    assert!(entries[0].placeable);
    assert!(!entries[0].enabled);
}

#[gpui::test]
fn every_change_raises_the_revision_and_notifies_observers(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = seen.clone();
    let _subscription =
        cx.update(|cx| cx.observe(&rig.host, move |_, _| counted.set(counted.get() + 1)));
    let start = rig.host.read_with(cx, |host, _| host.revision());

    rig.install(package(1), &[], cx).unwrap();
    cx.run_until_parked();
    let installed = rig.host.read_with(cx, |host, _| host.revision());
    rig.host.update(cx, |host, cx| {
        host.set_enabled(&plugin_id(), false, cx).unwrap()
    });
    cx.run_until_parked();
    let disabled = rig.host.read_with(cx, |host, _| host.revision());

    assert!(start < installed && installed < disabled);
    assert!(seen.get() >= 2);
}

#[gpui::test]
fn the_granted_capabilities_are_the_required_ones_and_the_accepted_optional(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    rig.install(package(1), &["monitor.usage"], cx).unwrap();

    let granted = rig.host.read_with(cx, |host, _| host.granted(&plugin_id()));

    assert!(granted.contains(&"clock.now".to_owned()));
    assert!(granted.contains(&"monitor.usage".to_owned()));
}

#[gpui::test]
fn the_values_of_a_plugin_start_at_the_defaults_of_its_schema(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.install(package(1), &[], cx).unwrap();

    let values = rig
        .host
        .read_with(cx, |host, _| host.values(&plugin_id()))
        .expect("the plugin is installed");

    assert!(values.get("city").is_some());
}

#[gpui::test]
fn a_package_is_read_without_storing_it(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.packages.offer("/tmp/clock.toml", &package(3));

    let read = rig
        .host
        .read_with(cx, |host, _| host.read_package("/tmp/clock.toml"));
    let missing = rig
        .host
        .read_with(cx, |host, _| host.read_package("/tmp/other.toml"));

    assert_eq!(
        read.map(|package| package.hash().to_string()).ok(),
        Some(package(3).hash().to_string())
    );
    assert_eq!(missing.err(), Some(PackageError::NotFound));
    assert!(!rig.packages.holds(&plugin_id()));
}

#[gpui::test]
fn the_audit_lists_what_happened_in_order(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.install(package(1), &[], cx).unwrap();
    rig.host.update(cx, |host, cx| {
        host.set_enabled(&plugin_id(), false, cx).unwrap()
    });

    let audit = rig.host.read_with(cx, |host, _| host.audit().to_vec());

    assert_eq!(audit.len(), 2);
    assert!(audit[0].seq < audit[1].seq);
}
