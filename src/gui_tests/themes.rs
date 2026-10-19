//! AppearanceSettings commands exercised through the real palette from a connected terminal.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, TestAppContext, WindowAppearance,
    component::{IndexPath, Theme},
    test::TestWindowExt as _,
};
use nocterm_ui::{ActiveDesign as _, TerminalStyle, hsla};
use nocterm_ui::{AppearanceMode, AppearanceSettings};

fn terminal_style(settings: &nocterm_store::SettingsHandle, cx: &gpui_kit::App) -> TerminalStyle {
    TerminalStyle::with_settings(settings.setting::<nocterm_ui::TerminalSettings>(cx), cx)
}

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

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
async fn theme_query_finds_both_commands_and_selection_restyles_the_connected_terminal(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _terminal, _, settings) = theme_fixture(cx);
    cx.update(|cx| {
        settings.update::<AppearanceSettings>(cx, |s| {
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
        assert_eq!(
            settings.setting::<AppearanceSettings>(cx).mode,
            AppearanceMode::Dark
        );
        assert_eq!(
            settings
                .setting::<AppearanceSettings>(cx)
                .dark_theme
                .as_deref(),
            Some("GUI Dark")
        );
        assert_eq!(
            settings
                .setting::<AppearanceSettings>(cx)
                .light_theme
                .as_deref(),
            Some("GUI Light")
        );
        let background = "#123456".parse().unwrap();
        assert_eq!(Theme::global(cx).background, hsla(background));
        assert_eq!(terminal_style(&settings, cx).background, background);
        assert_eq!(
            cx.design().dark.terminal.normal.red,
            "#ff0000".parse().unwrap()
        );
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
        assert_eq!(settings.setting::<AppearanceSettings>(cx).dark_theme, None);
        assert_eq!(cx.design().dark, nocterm_ui::DesignTokens::builtin().dark);
    });

    // The slot is fixed when opening, even if appearance changes in another view.
    palette(handle, "change theme", cx);
    enter(handle, cx);
    cx.update(|cx| settings.update::<AppearanceSettings>(cx, |s| s.mode = AppearanceMode::Light))
        .await
        .unwrap();
    frames(handle, cx);
    choose(handle, "GUI Dark", cx);
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<AppearanceSettings>(cx).mode,
            AppearanceMode::Light
        );
        assert_eq!(
            settings
                .setting::<AppearanceSettings>(cx)
                .dark_theme
                .as_deref(),
            Some("GUI Dark")
        );
        assert_eq!(
            settings
                .setting::<AppearanceSettings>(cx)
                .light_theme
                .as_deref(),
            Some("GUI Light")
        );
    });
    cx.update(|cx| settings.update::<AppearanceSettings>(cx, |s| s.light_theme = None))
        .await
        .unwrap();
    palette(handle, "change theme", cx);
    enter(handle, cx);
    choose(handle, "GUI Light", cx);
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<AppearanceSettings>(cx).mode,
            AppearanceMode::Light
        );
        assert_eq!(
            settings
                .setting::<AppearanceSettings>(cx)
                .light_theme
                .as_deref(),
            Some("GUI Light")
        );
        assert_eq!(
            terminal_style(&settings, cx).background,
            "#f1e2d3".parse().unwrap()
        );
    });
}

#[gpui_kit::test]
async fn palette_schemes_apply_all_modes_and_cancel_keeps_the_selected_themes(
    cx: &mut TestAppContext,
) {
    let (handle, _, _terminal, _, settings) = theme_fixture(cx);
    cx.update(|cx| {
        settings.update::<AppearanceSettings>(cx, |s| {
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
            assert_eq!(settings.setting::<AppearanceSettings>(cx).mode, mode);
            assert_eq!(
                settings
                    .setting::<AppearanceSettings>(cx)
                    .dark_theme
                    .as_deref(),
                Some("GUI Dark")
            );
            assert_eq!(
                settings
                    .setting::<AppearanceSettings>(cx)
                    .light_theme
                    .as_deref(),
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
            assert_eq!(terminal_style(&settings, cx).background, color);
        });
    }
    for query in ["color scheme", "change theme"] {
        let before = cx.update(|cx| settings.document(cx).clone());
        palette(handle, query, cx);
        enter(handle, cx);
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        frames(handle, cx);
        cx.update(|cx| assert_eq!(settings.document(cx), &before));
        cx.update_window(handle, |_, window, _| {
            assert!(window.try_find("dialog").is_none())
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn theme_commands_are_available_with_only_nocterm_default(cx: &mut TestAppContext) {
    let (handle, _, _, _, _, settings) =
        super::super::fixture_with_settings(cx, false, None, false, None);
    palette(handle, "change theme", cx);
    enter(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find(IndexPath::new(0)).visible());
        assert!(window.try_find(IndexPath::new(1)).is_none());
    })
    .unwrap();
    choose(handle, "Nocterm Default", cx);
    cx.update(|cx| {
        assert_eq!(settings.setting::<AppearanceSettings>(cx).dark_theme, None);
        assert_eq!(settings.setting::<AppearanceSettings>(cx).light_theme, None);
    });
}

#[gpui_kit::test]
fn appearance_command_reports_a_failed_save_without_changing_the_active_scheme(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, _, _, _, _, settings) = super::super::fixture_with_settings(
        cx,
        false,
        None,
        false,
        Some(nocterm_store::SettingsConfig::new(
            Default::default(),
            nocterm_settings::SettingsFile::new(directory.path()),
        )),
    );
    let before = cx.update(|cx| settings.document(cx).clone());
    let notices = cx
        .update_window(handle, |_, window, cx| {
            nocterm_ui::notice::count(window, cx)
        })
        .unwrap();
    palette(handle, "color scheme", cx);
    enter(handle, cx);
    choose(handle, "Dark", cx);
    frames(handle, cx);
    cx.update(|cx| assert_eq!(settings.document(cx), &before));
    cx.update_window(handle, |_, window, cx| {
        assert!(nocterm_ui::notice::count(window, cx) > notices)
    })
    .unwrap();
}

fn theme_fixture(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    gpui_kit::Entity<nocterm_workspace::Workspace>,
    gpui_kit::Entity<nocterm_terminal::TerminalView>,
    std::sync::Arc<super::super::MockTransport>,
    nocterm_store::SettingsHandle,
) {
    let backend = std::sync::Arc::new(nocterm_themes::test_support::InMemoryThemes::default());
    backend.user_document("test.json".into(),br##"{"themes":[{"name":"GUI Dark","appearance":"dark","style":{"background":"#123456","text":"#eeeeee","terminal.ansi.red":"#ff0000"}},{"name":"GUI Light","appearance":"light","style":{"background":"#f1e2d3","text":"#112233"}}]}"##.to_vec());
    super::super::fixture_with_themes(cx, backend)
}

struct CompetingThemeSurface;
impl gpui_kit::Render for CompetingThemeSurface {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

fn competing_theme_fixture(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, nocterm_store::SettingsHandle) {
    let (window, settings) = cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let service =
            nocterm_theme_service::ThemesHandle::install(None, None, Default::default(), cx);
        let projection = nocterm_ui::ThemeUi::install(
            settings.clone(),
            service,
            nocterm_ui::DesignTokens::builtin(),
            cx,
        );
        let (window, _) = gpui_kit::open_window(Default::default(), cx, |window, cx| {
            window
                .observe_window_appearance(move |_, cx| projection.apply(cx))
                .detach();
            cx.new(|_| CompetingThemeSurface)
        })
        .unwrap();
        (window, settings)
    });
    cx.update_window(window, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        nocterm_ui::open_scheme_picker(&settings, window, cx);
    })
    .unwrap();
    frames(window, cx);
    choose(window, "Light", cx);
    (window, settings)
}

#[gpui_kit::test]
async fn terminal_and_settings_paint_the_supplied_background_without_a_settings_alias(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _terminal, _transport, _, settings) =
        super::super::fixture_with_settings(cx, false, None, false, None);
    let (other_window, other) = competing_theme_fixture(cx);
    cx.update_window(other_window, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        settings.update::<AppearanceSettings>(cx, |s| {
            s.mode = AppearanceMode::Dark;
            s.layout = nocterm_ui::UiLayout::Classic;
        })
    })
    .await
    .unwrap();
    frames(handle, cx);
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<AppearanceSettings>(cx).mode,
            AppearanceMode::Dark
        );
        assert_eq!(
            other.setting::<AppearanceSettings>(cx).mode,
            AppearanceMode::Light
        );
    });
    let custom = nocterm_ui::DesignTokens::with_overrides(
        "[dark.terminal]\nbackground = '#123456'\n[dark.ui]\nbackground = '#abcdef'\n",
    )
    .unwrap();
    let expected = hsla("#123456".parse::<nocterm_ui::Color>().unwrap());
    cx.update(|cx| {
        cx.set_global(nocterm_ui::Design::new(custom));
    });
    frames(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .painted_quads()
                .iter()
                .any(|q| q.background.as_solid() == Some(expected)),
            "actual terminal background must be painted"
        );
        window.dispatch_action(Box::new(nocterm_workspace::OpenSettings), cx);
    })
    .unwrap();
    frames(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            workspace
                .read(cx)
                .find_item::<nocterm_settings_ui::SettingsView>()
                .is_some()
        );
        let bounds = window
            .find("settings-view")
            .bounds()
            .scale(window.scale_factor());
        assert!(
            window
                .painted_quads()
                .iter()
                .any(|q| q.bounds == bounds && q.background.as_solid() == Some(expected)),
            "the real Settings root paints its original terminal surface"
        );
    })
    .unwrap();
}
