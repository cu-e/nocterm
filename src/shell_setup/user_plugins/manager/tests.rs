//! What the User section's port refuses before it changes anything: a package that is not the
//! one the user looked at, and an installed plugin that is not the one the user agreed to
//! replace.
use std::{fs, path::Path, sync::Arc};

use gpui_kit::TestAppContext;
use nocterm_core::clock::{WallClock, WallTime};
use nocterm_plugins_ui::{InstallRequest, UserPluginError, UserPlugins as _};
use nocterm_store::ShutdownCoordinator;
use tempfile::TempDir;

use super::HostPlugins;
use crate::shell_setup::{UserShell, plugin_catalog};

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
    plugins: HostPlugins,
    directory: TempDir,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    let directory = TempDir::new().unwrap();
    let catalog = Arc::new(plugin_catalog().expect("the application catalog"));
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let shell = cx.update(|cx| {
        UserShell::install(
            &catalog,
            &paths,
            &ShutdownCoordinator::default(),
            Arc::new(Noon),
            cx,
        )
    });
    Rig {
        plugins: HostPlugins::new(shell.host().clone()),
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
