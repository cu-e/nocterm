use alloc::{string::ToString, vec::Vec};

use super::support::{manifest, snapshot, with_clock};
use crate::{
    ContentHash, DeclarativeRuntime, Effects, Event, LocalId, NodeKind, Package, PluginRuntime,
    RuntimeError,
};

fn package() -> Package {
    Package::new(ContentHash::new([3; 32]), manifest(), Vec::from(*b"bytes"))
}

#[test]
fn the_declarative_runtime_draws_views_from_snapshots() {
    let package = package();
    assert_eq!(package.bytes(), b"bytes");
    let instance = DeclarativeRuntime.instantiate(&package).unwrap();
    let local = LocalId::parse("face").unwrap();
    let mut snapshot = with_clock(snapshot());
    snapshot.settings.insert(
        "city".to_string(),
        nocterm_plugin_model::SettingValue::Text("Oslo".into()),
    );
    let view = instance.render(&local, &snapshot).unwrap();
    assert_eq!(view.nodes[2].kind, NodeKind::Text("14:05 Oslo".into()));
    assert_eq!(view.validate(), Ok(()));
}

#[test]
fn a_declarative_package_has_no_code_so_events_produce_nothing() {
    let instance = DeclarativeRuntime.instantiate(&package()).unwrap();
    let local = LocalId::parse("face").unwrap();
    let answer = instance
        .on_event(&local, Event::Activated { key: "go".into() }, &snapshot())
        .unwrap();
    assert_eq!(answer, Effects::default());
}

#[test]
fn an_unknown_contribution_is_a_runtime_error() {
    let instance = DeclarativeRuntime.instantiate(&package()).unwrap();
    let local = LocalId::parse("nothing").unwrap();
    assert_eq!(
        instance.render(&local, &snapshot()).unwrap_err(),
        RuntimeError::UnknownContribution(local.clone())
    );
    assert!(
        instance
            .on_event(&local, Event::SettingsChanged, &snapshot())
            .is_err()
    );
}
