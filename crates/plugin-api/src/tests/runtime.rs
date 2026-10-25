use alloc::{boxed::Box, string::ToString, sync::Arc, vec::Vec};

use super::support::{EXAMPLE, dto, host, manifest, snapshot, with_clock};
use crate::{
    ContentHash, DeclarativeRuntime, Effects, Event, LocalId, Manifest, NodeKind, Package,
    PluginInstance, PluginRuntime, Runtime, RuntimeError, Runtimes, View,
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

struct Marker;

impl PluginRuntime for Marker {
    fn instantiate(&self, _: &Package) -> Result<Box<dyn PluginInstance>, RuntimeError> {
        Ok(Box::new(Marked))
    }
}

struct Marked;

impl PluginInstance for Marked {
    fn render(&self, _: &LocalId, _: &crate::Snapshot) -> Result<View, RuntimeError> {
        Ok(View::default())
    }

    fn on_event(
        &self,
        _: &LocalId,
        _: Event,
        _: &crate::Snapshot,
    ) -> Result<Effects, RuntimeError> {
        Err(RuntimeError::Trap)
    }
}

fn code_package() -> Package {
    let text = EXAMPLE
        .replace("api = \"0.1\"", "api = \"0.1\"\nruntime = \"wasm\"")
        .replace(
            "[contribution.view]\nnodes = [\n  { kind = \"row\" },\n  { kind = \"icon\", parent = 0, icon = \"puzzle\" },\n  { kind = \"text\", parent = 0, text = \"{clock.now|hh-mm} {settings.city}\" },\n]\n",
            "",
        );
    let manifest = Manifest::validate(&dto(&text), &host()).unwrap();
    assert_eq!(manifest.runtime(), Runtime::Wasm);
    Package::new(ContentHash::new([4; 32]), manifest, Vec::from(*b"\0asm"))
}

#[test]
fn a_package_runs_on_the_runtime_its_manifest_names() {
    let runtimes = Runtimes::new().with_code(Arc::new(Marker));
    let local = LocalId::parse("face").unwrap();
    let declarative = runtimes.instantiate(&package()).unwrap();
    assert_eq!(
        declarative.on_event(&local, Event::SettingsChanged, &snapshot()),
        Ok(Effects::default())
    );
    let code = runtimes.instantiate(&code_package()).unwrap();
    assert_eq!(
        code.on_event(&local, Event::SettingsChanged, &snapshot()),
        Err(RuntimeError::Trap)
    );
}

#[test]
fn a_host_without_a_code_runtime_refuses_code_packages() {
    let runtimes = Runtimes::new();
    assert!(runtimes.instantiate(&package()).is_ok());
    assert_eq!(
        runtimes.instantiate(&code_package()).err(),
        Some(RuntimeError::NoRuntime(Runtime::Wasm))
    );
}
