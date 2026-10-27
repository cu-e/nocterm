//! What a run writes of the user's settings: an installation saves them, a developer session
//! only reads them.
use std::{fs, path::Path, sync::Arc};

use gpui_kit::TestAppContext;
use nocterm_core::Paths;
use nocterm_plugin_model::{ContributionId, PluginSet};
use nocterm_store::SettingsHandle;
use nocterm_window_layout::{EditOp, EditTarget, LayoutSettings, Subject, edit};
use tempfile::TempDir;

use super::load_settings;
use crate::shell_setup::{UserPluginMode, plugin_catalog};

/// A saved preset that places an installed plugin's widget on the right of the bar.
const SAVED: &str = r#"[layouts]
active = "p1"

[[layouts.presets]]
id = "p1"
name = "With a clock"

[layouts.presets.bar]
right = [{ item = "workspace.notifications" }, { item = "user-acme-clock.face" }]
"#;

/// Settings loaded for `mode` from a directory whose `settings.toml` holds `SAVED`.
fn open(mode: &UserPluginMode, cx: &mut TestAppContext) -> (TempDir, SettingsHandle) {
    let root = TempDir::new().unwrap();
    let paths = Paths::rooted_at(root.path());
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(paths.settings_file(), SAVED).unwrap();
    let mut config = load_settings(&paths, mode);
    config.register::<LayoutSettings>();
    let settings = cx.update(|cx| config.install(cx));
    (root, settings)
}

/// What the layout editor does when the user moves an item: it settles the preset against the
/// plugins the session knows, and these are the system plugins only.
fn move_an_item(settings: &SettingsHandle, cx: &mut TestAppContext) {
    let known = Arc::new(plugin_catalog().expect("the application catalog"));
    cx.update(|cx| {
        drop(settings.update::<LayoutSettings>(cx, move |layouts| {
            let op = EditOp::Move {
                subject: Subject::Contribution(ContributionId::new("workspace.notifications")),
                by: 1,
            };
            edit(
                layouts,
                false,
                &PluginSet::system(known),
                EditTarget::Selected,
                op,
            )
            .expect("the item can be moved");
        }));
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    cx.run_until_parked();
}

fn saved(root: &Path) -> String {
    fs::read_to_string(Paths::rooted_at(root).settings_file()).unwrap()
}

#[gpui_kit::test]
fn an_installation_saves_a_layout_edit_which_makes_the_control_meaningful(cx: &mut TestAppContext) {
    let (root, settings) = open(&UserPluginMode::Installed, cx);
    move_an_item(&settings, cx);

    assert_ne!(
        saved(root.path()),
        SAVED,
        "the same edit rewrites the file when settings are saved"
    );
}

#[cfg(feature = "user-plugins")]
#[gpui_kit::test]
fn a_developer_session_keeps_a_layout_edit_in_memory_and_leaves_the_saved_layouts_alone(
    cx: &mut TestAppContext,
) {
    let mode = UserPluginMode::Dev("focus_timer.wasm".into());
    let (root, settings) = open(&mode, cx);
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<LayoutSettings>(cx).presets[0].name,
            "With a clock",
            "the session reads the saved layouts"
        );
    });
    move_an_item(&settings, cx);

    assert_eq!(
        saved(root.path()),
        SAVED,
        "the installed plugin's widget stays placed in the file"
    );
    cx.update(|cx| {
        assert!(
            settings.setting::<LayoutSettings>(cx).presets[0]
                .tray
                .contains(&ContributionId::new("user-acme-clock.face")),
            "the session itself sees its edit: the plugin it does not know went to the tray"
        );
    });
}
