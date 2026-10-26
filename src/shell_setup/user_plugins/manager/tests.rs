//! What the User section's port refuses before it changes anything: a package that is not the
//! one the user looked at, and an installed plugin that is not the one the user agreed to
//! replace.
use std::{fs, path::Path, sync::Arc};

use gpui_kit::{Entity, TestAppContext};
use nocterm_core::clock::{WallClock, WallTime};
use nocterm_plugin_host::PluginHost;
use nocterm_plugin_model::{PluginId, SettingValue, SettingValues};
use nocterm_plugins_ui::{InstallRequest, UserPluginError, UserPlugins as _};
use nocterm_store::ShutdownCoordinator;
use tempfile::TempDir;

use super::HostPlugins;
use crate::shell_setup::{UserPluginMode, UserShell, plugin_catalog};

struct Noon;
impl WallClock for Noon {
    fn now(&self) -> WallTime {
        WallTime::default()
    }
}

fn clock_package(version_marker: &str) -> String {
    // The marker changes the bytes, hence the hash, without changing what the package is.
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/clock.toml"),
    )
    .expect("the example reads");
    format!("{text}\n# {version_marker}\n")
}

struct Rig {
    host: Entity<PluginHost>,
    plugins: HostPlugins,
    directory: TempDir,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    let directory = TempDir::new().unwrap();
    let catalog = Arc::new(plugin_catalog().expect("the application catalog"));
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let settings = super::super::gate::allowing(cx);
    let shell = cx.update(|cx| {
        UserShell::install(
            &catalog,
            &paths,
            &ShutdownCoordinator::default(),
            Arc::new(Noon),
            UserPluginMode::Installed,
            &settings,
            cx,
        )
    });
    let host = shell.host().expect("the shell loads user plugins").clone();
    Rig {
        plugins: HostPlugins::new(host.clone(), None),
        host,
        directory,
    }
}

impl Rig {
    fn file(&self, name: &str, text: &str) -> String {
        let path = self.directory.path().join(name);
        fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }

    /// What the consent would carry for the file as it is now.
    fn request(
        &self,
        location: &str,
        replaces: Option<String>,
        cx: &gpui_kit::App,
    ) -> InstallRequest {
        let candidate = self
            .plugins
            .inspect(location, cx)
            .expect("the package is acceptable");
        InstallRequest {
            location: location.to_owned(),
            hash: candidate.hash,
            replaces,
            accepted: Vec::new(),
            keep_data: false,
        }
    }
}

#[gpui_kit::test]
fn a_file_changed_after_the_consent_is_refused_and_nothing_is_installed(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let location = rig.file("clock.toml", &clock_package("looked at"));
    let request = cx.update(|cx| rig.request(&location, None, cx));
    rig.file("clock.toml", &clock_package("swapped after consent"));

    let outcome = cx.update(|cx| rig.plugins.install(request, cx));

    assert!(
        matches!(outcome, Err(UserPluginError::Changed)),
        "{outcome:?}"
    );
    assert!(cx.update(|cx| rig.plugins.rows(cx)).is_empty());
}

#[gpui_kit::test]
fn an_unchanged_file_installs_and_a_second_install_of_the_same_id_is_refused_as_stale(
    cx: &mut TestAppContext,
) {
    let rig = rig(cx);
    let location = rig.file("clock.toml", &clock_package("one"));
    let first = cx.update(|cx| rig.request(&location, None, cx));
    let second = cx.update(|cx| rig.request(&location, None, cx));

    let installed = cx.update(|cx| rig.plugins.install(first, cx));
    let raced = cx.update(|cx| rig.plugins.install(second, cx));

    assert!(installed.is_ok(), "{installed:?}");
    assert!(
        matches!(raced, Err(UserPluginError::InstalledChanged)),
        "a window that offered a fresh install must not replace what another window installed: {raced:?}"
    );
    assert_eq!(cx.update(|cx| rig.plugins.rows(cx)).len(), 1);
}

#[gpui_kit::test]
fn an_update_agreed_over_a_package_another_window_replaced_is_refused(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let first = rig.file("clock.toml", &clock_package("one"));
    cx.update(|cx| rig.plugins.install(rig.request(&first, None, cx), cx))
        .unwrap();
    let seen = cx.update(|cx| rig.plugins.rows(cx))[0].fingerprint.clone();
    let installed_hash = cx
        .update(|cx| rig.plugins.inspect(&first, cx))
        .unwrap()
        .replaces
        .expect("the same id is installed")
        .hash;
    assert!(installed_hash.starts_with(&seen));
    // Window A offers an update over package one.
    let update_a = rig.file("a.toml", &clock_package("two"));
    let request_a = cx.update(|cx| rig.request(&update_a, Some(installed_hash.clone()), cx));
    // Window B replaces package one first.
    let update_b = rig.file("b.toml", &clock_package("three"));
    let request_b = cx.update(|cx| rig.request(&update_b, Some(installed_hash), cx));
    cx.update(|cx| rig.plugins.install(request_b, cx)).unwrap();

    let outcome = cx.update(|cx| rig.plugins.install(request_a, cx));

    assert!(
        matches!(outcome, Err(UserPluginError::InstalledChanged)),
        "{outcome:?}"
    );
}

#[gpui_kit::test]
fn an_update_over_the_package_the_user_saw_replaces_it(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let first = rig.file("clock.toml", &clock_package("one"));
    cx.update(|cx| rig.plugins.install(rig.request(&first, None, cx), cx))
        .unwrap();
    let installed_hash = cx
        .update(|cx| rig.plugins.inspect(&first, cx))
        .unwrap()
        .replaces
        .unwrap()
        .hash;
    let update = rig.file("update.toml", &clock_package("two"));
    let request = cx.update(|cx| rig.request(&update, Some(installed_hash), cx));

    let outcome = cx.update(|cx| rig.plugins.install(request, cx));

    assert!(outcome.is_ok(), "{outcome:?}");
}

#[gpui_kit::test]
fn a_refusal_of_the_host_keeps_its_cause_for_the_section(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let id = nocterm_plugin_model::PluginId::owned("user-example-nobody");

    let outcome = cx.update(|cx| rig.plugins.set_enabled(&id, true, cx));

    let Err(error) = outcome else {
        panic!("an unknown plugin cannot be enabled");
    };
    assert!(
        std::error::Error::source(&error).is_some(),
        "the typed cause is kept, not flattened to text"
    );
}

/// Lets the persistence service write what the host handed it.
fn saved(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    cx.run_until_parked();
}

impl Rig {
    /// Installs the clock example and says which plugin it is.
    fn installed(&self, cx: &mut TestAppContext) -> nocterm_plugin_model::PluginId {
        let location = self.file("clock.toml", &clock_package("installed"));
        cx.update(|cx| self.plugins.install(self.request(&location, None, cx), cx))
            .expect("the package installs");
        cx.update(|cx| self.plugins.rows(cx))[0].id.clone()
    }

    fn actions(&self, cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| self.plugins.audit(cx))
            .into_iter()
            .map(|line| line.action)
            .collect()
    }

    /// The user sets the clock's label, which the host stores with the plugin's data.
    fn set_label(&self, id: &PluginId, label: &str, cx: &mut TestAppContext) {
        let mut values = SettingValues::default();
        values.insert("label", SettingValue::Text(label.into()));
        self.host
            .update(cx, |host, cx| host.replace_values(id, &values, cx))
            .expect("the values are stored");
        saved(cx);
    }

    /// The label the clock would show now.
    fn label(&self, id: &PluginId, cx: &mut TestAppContext) -> Option<SettingValue> {
        self.host
            .read_with(cx, |host, _| host.values(id))
            .and_then(|values| values.get("label").cloned())
    }

    fn row(&self, cx: &mut TestAppContext) -> nocterm_plugins_ui::UserPluginRow {
        cx.update(|cx| self.plugins.rows(cx)).remove(0)
    }
}

#[gpui_kit::test]
fn an_installed_plugin_is_listed_with_what_the_host_knows_of_it(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let id = rig.installed(cx);

    let row = rig.row(cx);

    assert_eq!(row.id, id);
    assert_eq!(row.name, "Example Clock");
    assert_eq!(row.publisher, "example");
    assert_eq!(row.contributions, ["Time"]);
    assert_eq!(row.granted, ["See the current time and your time zone."]);
    assert_eq!(row.fingerprint.len(), 16);
    assert!(matches!(row.state, nocterm_plugins_ui::RowState::Ready));
    assert_eq!(rig.actions(cx), ["Installed"]);
}

#[gpui_kit::test]
fn enabling_and_disabling_change_the_row_and_leave_audit_lines(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let id = rig.installed(cx);
    let enabled = rig.row(cx).enabled;

    cx.update(|cx| rig.plugins.set_enabled(&id, !enabled, cx))
        .unwrap();
    assert_eq!(rig.row(cx).enabled, !enabled);
    cx.update(|cx| rig.plugins.set_enabled(&id, enabled, cx))
        .unwrap();
    assert_eq!(rig.row(cx).enabled, enabled);

    let actions = rig.actions(cx);
    assert_eq!(actions.len(), 3, "{actions:?}");
    assert!(actions.contains(&"Turned off".to_owned()), "{actions:?}");
    assert!(actions.contains(&"Turned on".to_owned()), "{actions:?}");
}

#[gpui_kit::test]
fn placement_is_granted_and_revoked_through_the_port(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let id = rig.installed(cx);

    cx.update(|cx| rig.plugins.set_placeable(&id, true, cx))
        .unwrap();
    assert!(rig.row(cx).placeable);
    cx.update(|cx| rig.plugins.set_placeable(&id, false, cx))
        .unwrap();
    assert!(!rig.row(cx).placeable);

    let actions = rig.actions(cx);
    assert!(
        actions.contains(&"Allowed in windows".to_owned()),
        "{actions:?}"
    );
    assert!(
        actions.contains(&"Revoked from windows".to_owned()),
        "{actions:?}"
    );
}

#[gpui_kit::test]
fn removing_a_plugin_keeps_its_data_for_a_later_install_unless_the_user_removes_it_too(
    cx: &mut TestAppContext,
) {
    let rig = rig(cx);
    let kept = SettingValue::Text("Kept".into());
    for (with_data, expected) in [(false, Some(&kept)), (true, None)] {
        let id = rig.installed(cx);
        rig.set_label(&id, "Kept", cx);
        assert_eq!(rig.label(&id, cx).as_ref(), Some(&kept));

        cx.update(|cx| rig.plugins.remove(&id, with_data, cx))
            .unwrap();
        saved(cx);
        assert!(cx.update(|cx| rig.plugins.rows(cx)).is_empty());

        let again = rig.installed(cx);
        assert_eq!(
            rig.label(&again, cx)
                .as_ref()
                .filter(|label| **label == kept),
            expected,
            "removing with the data: {with_data}"
        );
        cx.update(|cx| rig.plugins.remove(&again, true, cx))
            .unwrap();
    }
}

#[gpui_kit::test]
fn removing_only_the_data_resets_the_values_and_leaves_the_plugin_installed(
    cx: &mut TestAppContext,
) {
    let rig = rig(cx);
    let id = rig.installed(cx);
    rig.set_label(&id, "Kept", cx);

    cx.update(|cx| rig.plugins.remove_data(&id, cx)).unwrap();

    assert_eq!(cx.update(|cx| rig.plugins.rows(cx)).len(), 1);
    assert_ne!(
        rig.label(&id, cx),
        Some(SettingValue::Text("Kept".into())),
        "the value the user set is gone"
    );
    assert!(rig.actions(cx).contains(&"Data removed".to_owned()));
}

#[gpui_kit::test]
fn every_act_on_an_unknown_plugin_is_refused_with_its_cause(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let id = nocterm_plugin_model::PluginId::owned("user-example-nobody");
    let refused = [
        cx.update(|cx| rig.plugins.set_placeable(&id, true, cx)),
        cx.update(|cx| rig.plugins.remove(&id, false, cx)),
        cx.update(|cx| rig.plugins.remove(&id, true, cx)),
        cx.update(|cx| rig.plugins.remove_data(&id, cx)),
    ];

    for outcome in refused {
        let Err(error) = outcome else {
            panic!("an unknown plugin cannot be acted on");
        };
        assert!(matches!(error, UserPluginError::Manage(_)), "{error:?}");
        assert!(std::error::Error::source(&error).is_some());
    }
    assert!(rig.actions(cx).is_empty());
}

#[gpui_kit::test]
fn a_change_is_announced_to_the_observer_until_it_is_dropped(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let id = rig.installed(cx);
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let subscription = cx.update(|cx| {
        let seen = seen.clone();
        rig.plugins
            .observe(cx, Box::new(move |_| seen.set(seen.get() + 1)))
    });

    cx.update(|cx| rig.plugins.set_placeable(&id, true, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(seen.get() > 0, "the section learns that the list changed");

    drop(subscription);
    let before = seen.get();
    cx.update(|cx| rig.plugins.set_placeable(&id, false, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(seen.get(), before);
}
