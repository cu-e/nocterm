//! The start gate: a run loads user plugins only when `[plugins] user_plugins` allows them and
//! it was not started in safe mode. A held run opens no file, publishes only the system plugins
//! and attaches nothing to its windows.
use std::{fs, sync::Arc};

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
    let state = TempDir::new().unwrap();
    // A records file from an earlier run: a held run must leave it exactly as it is.
    let root = state.path().join("state").join("plugins");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("records.json"), "earlier run").unwrap();
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

impl Run {
    fn records(&self) -> String {
        let path = self
            .state
            .path()
            .join("state")
            .join("plugins")
            .join("records.json");
        fs::read_to_string(path).expect("the earlier file is still there")
    }

    fn user_plugins_published(&self, cx: &mut TestAppContext) -> usize {
        let source = self.shell.source(&self.catalog);
        cx.update(|cx| source.current(cx).users().len())
    }
}

fn assert_held(run: &Run, by: HeldBy, cx: &mut TestAppContext) {
    assert_eq!(run.shell.held_by(), Some(by));
    assert!(run.shell.host().is_none(), "no host entity exists");
    assert!(
        run.shell.facts().is_none(),
        "the bar editor learns no facts"
    );
    assert_eq!(run.user_plugins_published(cx), 0);
    assert_eq!(run.records(), "earlier run", "no stored file was opened");
    let directory = run.shell.directory(PluginSettingsDirectory::new());
    assert!(
        cx.update(|cx| directory.get(&PluginId::owned("user-example-clock"), cx))
            .is_none()
    );
}

#[gpui_kit::test]
fn a_run_without_the_switch_loads_nothing_and_touches_no_file(cx: &mut TestAppContext) {
    let run = start(UserPluginMode::Installed, &settings("", cx), cx);

    assert_held(&run, HeldBy::NotAllowed, cx);
}

#[gpui_kit::test]
fn a_switch_that_is_off_holds_the_run(cx: &mut TestAppContext) {
    let run = start(
        UserPluginMode::Installed,
        &settings("[plugins]\nuser_plugins = false\n", cx),
        cx,
    );

    assert_held(&run, HeldBy::NotAllowed, cx);
}

#[gpui_kit::test]
fn a_plugins_table_that_does_not_read_holds_the_run(cx: &mut TestAppContext) {
    let broken = "[plugins]\nuser_plugins = true\ndisabled = [\"monitor\", 3]\n";
    let run = start(UserPluginMode::Installed, &settings(broken, cx), cx);

    assert_held(&run, HeldBy::NotAllowed, cx);
}

#[gpui_kit::test]
fn safe_mode_wins_over_an_allowed_switch(cx: &mut TestAppContext) {
    let run = start(UserPluginMode::SafeMode, &allowing(cx), cx);

    assert_held(&run, HeldBy::SafeMode, cx);
}

#[gpui_kit::test]
fn an_allowed_run_opens_the_stored_plugins(cx: &mut TestAppContext) {
    let run = start(UserPluginMode::Installed, &allowing(cx), cx);

    assert_eq!(run.shell.held_by(), None);
    assert!(run.shell.host().is_some());
    assert!(run.shell.facts().is_some());
}

#[gpui_kit::test]
fn a_developer_session_is_not_held_by_the_switch(cx: &mut TestAppContext) {
    let run = start(
        UserPluginMode::Dev("clock.toml".into()),
        &settings("", cx),
        cx,
    );

    assert_eq!(run.shell.held_by(), None);
    assert!(run.shell.host().is_some());
    assert_eq!(run.records(), "earlier run", "a session saves nothing");
}
