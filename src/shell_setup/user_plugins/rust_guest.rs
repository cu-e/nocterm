//! A code plugin written with the SDK and built for wasm32 runs through the same check and
//! runtime an installed package does. The module is not checked in: CI builds
//! `examples/plugins/focus-timer` and names the file in `NOCTERM_GUEST_WASM`. Without the
//! variable the test passes vacuously, unless `NOCTERM_REQUIRE_WASM_GUEST` is set (CI), like the
//! `NOCTERM_REQUIRE_SSHD` precedent.
use std::{fs, path::PathBuf};

use nocterm_plugin_api::{
    ContributionContext, Effect, Event, Kind, LocalId, Observed, PluginRuntime as _, Snapshot,
    Zone, capability::clock::ClockNow,
};
use nocterm_plugin_wasm::WasmRuntime;
use tempfile::TempDir;

use super::{check::check_directory, runtime::DiscardGuestLog};
use crate::shell_setup::plugin_catalog;

/// The built example, or `None` when the environment does not name one.
fn guest() -> Option<PathBuf> {
    match std::env::var_os("NOCTERM_GUEST_WASM") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            assert!(
                std::env::var_os("NOCTERM_REQUIRE_WASM_GUEST").is_none(),
                "NOCTERM_REQUIRE_WASM_GUEST is set but NOCTERM_GUEST_WASM names no module"
            );
            None
        }
    }
}

fn snapshot(kind: Kind) -> Snapshot {
    Snapshot::new(ContributionContext {
        kind,
        zone: Zone::Right,
    })
}

#[test]
fn a_rust_guest_passes_the_check_and_answers_events_through_the_runtime() {
    let Some(module) = guest() else {
        return;
    };
    let directory = TempDir::new().unwrap();
    fs::copy(&module, directory.path().join("focus_timer.wasm"))
        .expect("the built example is readable");

    let catalog = plugin_catalog().expect("the application catalog");
    let report = check_directory(directory.path(), &catalog);
    let text = report.to_string();
    assert!(report.passed(), "{text}");
    assert!(
        text.contains("Example Focus Timer"),
        "the manifest section survived the build: {text}"
    );

    let package =
        nocterm_plugin_packages::read_package(&directory.path().join("focus_timer.wasm"), &[])
            .expect("the module is a package");
    let runtime = WasmRuntime::new(std::sync::Arc::new(DiscardGuestLog));
    let instance = runtime.instantiate(&package).expect("the module links");

    let bar = LocalId::parse("bar").unwrap();
    let mut running = snapshot(Kind::Widget);
    running.observations.insert(
        "clock.now".into(),
        Ok(Observed::Clock(ClockNow {
            unix_minutes: 1_012,
            utc_offset_seconds: 0,
        })),
    );
    running.storage.insert("start".into(), "1000".into());
    let view = instance.render(&bar, &running).expect("the bar renders");
    assert!(
        format!("{view:?}").contains("Focus 12m"),
        "the guest read the clock and its stored start: {view:?}"
    );

    let effects = instance
        .on_event(&bar, Event::Activated { key: "open".into() }, &running)
        .expect("the guest answers the event");
    assert!(
        effects
            .0
            .iter()
            .any(|request| matches!(&request.effect, Effect::PanelsSetOpen(set) if set.open)),
        "activating the bar asks to open the panel: {effects:?}"
    );
}
