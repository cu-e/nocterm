//! Appearance commands exercised through the real palette from a connected terminal.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, TestAppContext, WindowAppearance,
    component::{IndexPath, Theme},
    test::TestWindowExt as _,
};
use nocterm_settings::{Appearance, AppearanceMode};
use nocterm_ui::{ActiveDesign as _, SettingsExt as _, TerminalStyle, hsla};

use super::super::fixture;

fn frames(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn palette(handle: AnyWindowHandle, query: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-shift-p"
            } else {
                "ctrl-shift-p"
            },
            cx,
        );
    })
    .unwrap();
    frames(handle, cx);
    cx.simulate_input(handle, query);
    frames(handle, cx);
}

fn enter(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    frames(handle, cx);
}

fn choose(handle: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
    cx.simulate_input(handle, text);
    frames(handle, cx);
    enter(handle, cx);
}

fn install_catalog(cx: &mut TestAppContext) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let dirs = nocterm_themes::ThemeDirs {
        user: root.path().join("user"),
        installed: root.path().join("installed"),
    };
    std::fs::create_dir(&dirs.user).unwrap();
    std::fs::write(
        dirs.user.join("test.json"),
        r##"{"themes":[{"name":"GUI Dark","appearance":"dark","style":{"background":"#123456","text":"#eeeeee","terminal.ansi.red":"#ff0000"}},{"name":"GUI Light","appearance":"light","style":{"background":"#f1e2d3","text":"#112233"}}]}"##,
    )
    .unwrap();
    let catalog = nocterm_themes::ThemeCatalog::load(&dirs);
    cx.update(|cx| nocterm_ui::init_themes(dirs, catalog, cx));
    root
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
async fn theme_query_finds_both_commands_and_selection_restyles_the_connected_terminal(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _, _) = fixture(cx);
    let _directory = install_catalog(cx);
    cx.update(|cx| {
        cx.update_setting::<Appearance>(|s| {
            s.mode = AppearanceMode::Dark;
            s.light_theme = Some("GUI Light".into());
        })
    })
    .await
    .unwrap();
    frames(handle, cx);
    let indices = cx
        .update_window(handle, |_, window, cx| {
            let mut names: Vec<_> = window
                .available_actions(cx)
                .into_iter()
                .filter(|a| nocterm_keymap::offered(a.name()))
                .map(|a| nocterm_keymap::humanize(a.name()))
                .collect();
            names.sort();
            names.dedup();
            ["Change Color Scheme", "Change Theme"].map(|command| {
                names
                    .iter()
                    .position(|name| name == &format!("Workspace: {command}"))
                    .expect("appearance action registered over the terminal")
            })
        })
        .unwrap();
    palette(handle, "theme", cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find(IndexPath::new(indices[0])).visible());
        assert!(window.find(IndexPath::new(indices[1])).visible());
        window.press("down", cx);
    })
    .unwrap();
    enter(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find(IndexPath::new(1)).visible());
        assert!(window.try_find(IndexPath::new(2)).is_none());
    })
    .unwrap();
    choose(handle, "GUI Dark", cx);
    cx.update(|cx| {
        assert_eq!(cx.setting::<Appearance>().mode, AppearanceMode::Dark);
        assert_eq!(
            cx.setting::<Appearance>().dark_theme.as_deref(),
            Some("GUI Dark")
        );
        assert_eq!(
            cx.setting::<Appearance>().light_theme.as_deref(),
            Some("GUI Light")
        );
        let background = "#123456".parse().unwrap();
        assert_eq!(Theme::global(cx).background, hsla(background));
        assert_eq!(TerminalStyle::current(cx).background, background);
        assert_eq!(cx.design().dark.terminal.red, "#ff0000".parse().unwrap());
        assert_eq!(
            workspace.read(cx).items().count(),
            1,
            "no Settings tab needed"
        );
    });

    palette(handle, "change theme", cx);
    enter(handle, cx);
    choose(handle, "Nocterm Default", cx);
    cx.update(|cx| {
        assert_eq!(cx.setting::<Appearance>().dark_theme, None);
        assert_eq!(cx.design().dark, nocterm_ui::DesignTokens::builtin().dark);
    });

    // The slot is fixed when opening, even if appearance changes in another view.
    palette(handle, "change theme", cx);
    enter(handle, cx);
    cx.update(|cx| cx.update_setting::<Appearance>(|s| s.mode = AppearanceMode::Light))
        .await
        .unwrap();
    frames(handle, cx);
    choose(handle, "GUI Dark", cx);
    cx.update(|cx| {
        assert_eq!(cx.setting::<Appearance>().mode, AppearanceMode::Light);
        assert_eq!(
            cx.setting::<Appearance>().dark_theme.as_deref(),
            Some("GUI Dark")
        );
        assert_eq!(
            cx.setting::<Appearance>().light_theme.as_deref(),
            Some("GUI Light")
        );
    });
    cx.update(|cx| cx.update_setting::<Appearance>(|s| s.light_theme = None))
        .await
        .unwrap();
    palette(handle, "change theme", cx);
    enter(handle, cx);
    choose(handle, "GUI Light", cx);
    cx.update(|cx| {
        assert_eq!(cx.setting::<Appearance>().mode, AppearanceMode::Light);
        assert_eq!(
            cx.setting::<Appearance>().light_theme.as_deref(),
            Some("GUI Light")
        );
        assert_eq!(
            TerminalStyle::current(cx).background,
            "#f1e2d3".parse().unwrap()
        );
    });
}

#[gpui_kit::test]
async fn palette_schemes_apply_all_modes_and_cancel_keeps_the_selected_themes(
    cx: &mut TestAppContext,
) {
    let (handle, _, _, _) = fixture(cx);
    let _directory = install_catalog(cx);
    cx.update(|cx| {
        cx.update_setting::<Appearance>(|s| {
            s.dark_theme = Some("GUI Dark".into());
            s.light_theme = Some("GUI Light".into());
        })
    })
    .await
    .unwrap();
    for (label, mode) in [
        ("Light", AppearanceMode::Light),
        ("Dark", AppearanceMode::Dark),
        ("System", AppearanceMode::System),
    ] {
        palette(handle, "color scheme", cx);
        enter(handle, cx);
        choose(handle, label, cx);
        cx.update(|cx| {
            assert_eq!(cx.setting::<Appearance>().mode, mode);
            assert_eq!(
                cx.setting::<Appearance>().dark_theme.as_deref(),
                Some("GUI Dark")
            );
            assert_eq!(
                cx.setting::<Appearance>().light_theme.as_deref(),
                Some("GUI Light")
            );
            let dark = match mode {
                AppearanceMode::Dark => true,
                AppearanceMode::Light => false,
                AppearanceMode::System => matches!(
                    cx.window_appearance(),
                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                ),
            };
            let color = if dark { "#123456" } else { "#f1e2d3" }.parse().unwrap();
            assert_eq!(TerminalStyle::current(cx).background, color);
        });
    }
    for query in ["color scheme", "change theme"] {
        let before = cx.update(|cx| cx.global::<nocterm_ui::SettingsStore>().document().clone());
        palette(handle, query, cx);
        enter(handle, cx);
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        frames(handle, cx);
        cx.update(|cx| assert_eq!(cx.global::<nocterm_ui::SettingsStore>().document(), &before));
        cx.update_window(handle, |_, window, _| {
            assert!(window.try_find("dialog").is_none())
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn theme_commands_are_available_with_only_nocterm_default(cx: &mut TestAppContext) {
    let (handle, _, _, _) = fixture(cx);
    palette(handle, "change theme", cx);
    enter(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find(IndexPath::new(0)).visible());
        assert!(window.try_find(IndexPath::new(1)).is_none());
    })
    .unwrap();
    choose(handle, "Nocterm Default", cx);
    cx.update(|cx| {
        assert_eq!(cx.setting::<Appearance>().dark_theme, None);
        assert_eq!(cx.setting::<Appearance>().light_theme, None);
    });
}

#[gpui_kit::test]
fn appearance_command_reports_a_failed_save_without_changing_the_active_scheme(
    cx: &mut TestAppContext,
) {
    let (handle, _, _, _) = fixture(cx);
    let directory = tempfile::tempdir().unwrap();
    let before = cx.update(|cx| {
        let settings = cx.global::<nocterm_ui::SettingsStore>().document().clone();
        cx.set_global(nocterm_ui::SettingsStore::new(
            settings.clone(),
            nocterm_settings::SettingsFile::new(directory.path()),
        ));
        settings
    });
    let notices = cx
        .update_window(handle, |_, window, cx| {
            nocterm_ui::notice::count(window, cx)
        })
        .unwrap();
    palette(handle, "color scheme", cx);
    enter(handle, cx);
    choose(handle, "Dark", cx);
    cx.update(|cx| assert_eq!(cx.global::<nocterm_ui::SettingsStore>().document(), &before));
    cx.update_window(handle, |_, window, cx| {
        assert!(nocterm_ui::notice::count(window, cx) > notices)
    })
    .unwrap();
}
