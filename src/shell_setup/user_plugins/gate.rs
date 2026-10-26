//! The start gate: a run loads user plugins only when `[plugins] user_plugins` allows them and
//! it was not started in safe mode. A held run opens no file, publishes only the system plugins
//! and attaches nothing to its windows.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use gpui_kit::TestAppContext;
use nocterm_core::clock::{WallClock, WallTime};
use nocterm_plugin_model::{PluginCatalog, PluginId};
use nocterm_plugins_ui::HeldBy;
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SettingsHandle, ShutdownCoordinator};
use nocterm_ui::PluginSettingsDirectory;
use tempfile::TempDir;

use super::UserShell;
use crate::shell_setup::{UserPluginMode, plugin_catalog};

/// The clock the gate tests run with.
pub(super) struct Noon;
impl WallClock for Noon {
    fn now(&self) -> WallTime {
        WallTime::default()
    }
}

/// Settings that allow the user plugins, as the user does on the Plugins page.
pub(super) fn allowing(cx: &mut TestAppContext) -> SettingsHandle {
    settings("[plugins]\nuser_plugins = true\n", cx)
}

fn settings(text: &str, cx: &mut TestAppContext) -> SettingsHandle {
    let document = SettingsDocument::from_table(text.parse().expect("toml"));
    cx.update(|cx| SettingsConfig::in_memory(document).install(cx))
}

struct Run {
    shell: UserShell,
    catalog: Arc<PluginCatalog>,
    state: TempDir,
}

fn start(mode: UserPluginMode, settings: &SettingsHandle, cx: &mut TestAppContext) -> Run {
    start_in(TempDir::new().unwrap(), mode, settings, cx)
}

fn start_in(
    state: TempDir,
    mode: UserPluginMode,
    settings: &SettingsHandle,
    cx: &mut TestAppContext,
) -> Run {
    let catalog = Arc::new(plugin_catalog().expect("the application catalog"));
    let paths = nocterm_core::Paths::rooted_at(state.path());
    let shell = cx.update(|cx| {
        UserShell::install(
            &catalog,
            &paths,
            &ShutdownCoordinator::default(),
            Arc::new(Noon),
            mode,
            settings,
            cx,
        )
    });
    cx.run_until_parked();
    Run {
        shell,
        catalog,
        state,
    }
}

/// A state directory an earlier run left behind: the records file, the stored package and the
/// data of the example clock, as an allowed run writes them.
fn installation(cx: &mut TestAppContext) -> TempDir {
    let run = start(UserPluginMode::Installed, &allowing(cx), cx);
    let clock = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/plugins/clock.toml");
    cx.update(|cx| {
        run.shell
            .host()
            .expect("an allowed run has a host")
            .update(cx, |host, cx| {
                let package = host.read_package(clock).expect("the example reads");
                host.install(package, &[], cx)
                    .expect("the example installs");
            });
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    let files = files_under(&run.plugins_dir());
    assert!(
        files.keys().any(|path| path.ends_with("records.json")),
        "the earlier run stored its records: {files:?}"
    );
    run.state
}

/// Every file and directory under `root` with its bytes and modification time, so that a run
/// which touched any of them, or added one, shows.
fn files_under(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        let bytes = if metadata.is_dir() {
            for entry in fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
            Vec::new()
        } else {
            fs::read(&path).unwrap()
        };
        found.insert(path, (bytes, metadata.modified().unwrap()));
    }
    found
}

impl Run {
    fn plugins_dir(&self) -> PathBuf {
        self.state.path().join("state").join("plugins")
    }

    fn user_plugins_published(&self, cx: &mut TestAppContext) -> usize {
        let source = self.shell.source(&self.catalog);
        cx.update(|cx| source.current(cx).users().len())
    }
}

/// A run over `state` in `mode` that must not open it: it publishes no user plugin, builds no
/// host, and every file under the plugin directory is as the earlier run left it.
fn assert_held(
    state: TempDir,
    mode: UserPluginMode,
    settings: &SettingsHandle,
    cx: &mut TestAppContext,
) -> Run {
    let before = files_under(&state.path().join("state").join("plugins"));
    let run = start_in(state, mode, settings, cx);
    assert!(run.shell.held_by().is_some());
    assert!(run.shell.host().is_none(), "no host entity exists");
    assert!(
        run.shell.facts().is_none(),
        "the bar editor learns no facts"
    );
    assert_eq!(run.user_plugins_published(cx), 0);
    assert_eq!(
        files_under(&run.plugins_dir()),
        before,
        "no stored file was opened, changed or added"
    );
    let directory = run.shell.directory(PluginSettingsDirectory::new());
    assert!(
        cx.update(|cx| directory.get(&PluginId::owned("user-example-clock"), cx))
            .is_none()
    );
    run
}

#[gpui_kit::test]
fn the_same_installation_loads_when_allowed(cx: &mut TestAppContext) {
    let state = installation(cx);

    let run = start_in(state, UserPluginMode::Installed, &allowing(cx), cx);

    assert_eq!(run.shell.held_by(), None);
    assert!(run.shell.host().is_some());
    assert!(run.shell.facts().is_some());
    assert_eq!(
        run.user_plugins_published(cx),
        1,
        "the stored clock is published, so the held runs below hold something back"
    );
}

#[gpui_kit::test]
fn a_run_without_the_switch_loads_nothing_and_touches_no_file(cx: &mut TestAppContext) {
    let run = assert_held(
        installation(cx),
        UserPluginMode::Installed,
        &settings("", cx),
        cx,
    );

    assert_eq!(run.shell.held_by(), Some(HeldBy::NotAllowed));
}

#[gpui_kit::test]
fn a_switch_that_is_off_holds_the_run(cx: &mut TestAppContext) {
    let run = assert_held(
        installation(cx),
        UserPluginMode::Installed,
        &settings("[plugins]\nuser_plugins = false\n", cx),
        cx,
    );

    assert_eq!(run.shell.held_by(), Some(HeldBy::NotAllowed));
}

#[gpui_kit::test]
fn a_plugins_table_that_does_not_read_holds_the_run_and_names_the_reason(cx: &mut TestAppContext) {
    let broken = "[plugins]\nuser_plugins = true\ndisabled = [\"monitor\", 3]\n";
    let run = assert_held(
        installation(cx),
        UserPluginMode::Installed,
        &settings(broken, cx),
        cx,
    );

    let Some(HeldBy::Unreadable { reason }) = run.shell.held_by() else {
        panic!("held for another reason: {:?}", run.shell.held_by());
    };
    assert!(!reason.is_empty());
}

#[gpui_kit::test]
fn safe_mode_wins_over_an_allowed_switch(cx: &mut TestAppContext) {
    let run = assert_held(
        installation(cx),
        UserPluginMode::SafeMode,
        &allowing(cx),
        cx,
    );

    assert_eq!(run.shell.held_by(), Some(HeldBy::SafeMode));
}

#[gpui_kit::test]
fn a_held_run_on_a_fresh_machine_creates_no_plugin_directory(cx: &mut TestAppContext) {
    for (mode, text) in [
        (UserPluginMode::Installed, ""),
        (UserPluginMode::SafeMode, "[plugins]\nuser_plugins = true\n"),
    ] {
        let run = start(mode, &settings(text, cx), cx);

        assert!(run.shell.host().is_none());
        assert!(
            !run.plugins_dir().exists(),
            "a held run creates no records, data or package directory"
        );
    }
}

#[gpui_kit::test]
fn a_developer_session_is_not_held_by_the_switch_and_saves_nothing(cx: &mut TestAppContext) {
    let state = installation(cx);
    let before = files_under(&state.path().join("state").join("plugins"));

    let run = start_in(
        state,
        UserPluginMode::Dev("clock.toml".into()),
        &settings("", cx),
        cx,
    );

    assert_eq!(run.shell.held_by(), None);
    assert!(run.shell.host().is_some());
    assert_eq!(
        run.user_plugins_published(cx),
        0,
        "nothing installed loads in a session"
    );
    assert_eq!(files_under(&run.plugins_dir()), before);
}
