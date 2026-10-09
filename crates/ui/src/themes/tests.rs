use super::*;
use crate::{ActiveDesign, TerminalStyle, hsla, init};
use gpui_kit::{TestAppContext, component::Theme};
use nocterm_settings::{Appearance as AppearanceSettings, AppearanceMode, SettingsDocument};
#[gpui_kit::test]
async fn selection_applies_ui_and_terminal_then_clears_without_a_loop(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dirs = ThemeDirs {
        user: root.path().join("user"),
        installed: root.path().join("installed"),
    };
    std::fs::create_dir(&dirs.user).unwrap();
    std::fs::write(dirs.user.join("test.json"),r##"{"themes":[{"name":"Custom","appearance":"dark","style":{"editor.background":"#112233","text":"#eeeeee","terminal.ansi.red":"#ff0000"}}]}"##).unwrap();
    let base = DesignTokens::with_overrides("[dark.terminal]\nblue = '#abcdef'\n").unwrap();
    let expected = base.clone();
    cx.update(|cx| {
        gpui_kit::init(cx);
        let appearance = AppearanceSettings {
            mode: AppearanceMode::Dark,
            ..AppearanceSettings::default()
        };
        let settings = SettingsStore::in_memory(SettingsDocument::default()).with(appearance);
        init(base, settings, cx);
        init_themes(dirs.clone(), ThemeCatalog::load(&dirs), cx);
    });
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    cx.update(|cx| {
        let count = notifications.clone();
        cx.observe_global::<Design>(move |_| count.set(count.get() + 1))
            .detach();
    });
    cx.update(|cx| {
        cx.update_setting::<AppearanceSettings>(|s| s.dark_theme = Some("Custom".into()))
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            Theme::global(cx).background,
            hsla("#112233".parse().unwrap())
        );
        assert_eq!(
            TerminalStyle::current(cx).background,
            "#112233".parse().unwrap()
        );
        assert_eq!(cx.design().dark.terminal.red, "#ff0000".parse().unwrap());
        assert_eq!(
            cx.design().dark.terminal.blue,
            DesignTokens::builtin().dark.terminal.blue
        );
        assert!(cx.global::<Design>().is_imported(true));
        assert!(!cx.global::<Design>().is_imported(false));
    });
    assert_eq!(notifications.get(), 1);
    for name in [Some("Missing".to_owned()), None] {
        cx.update(|cx| cx.update_setting::<AppearanceSettings>(move |s| s.dark_theme = name))
            .await
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.design(), &expected));
    }
    cx.update(|cx| {
        cx.update_setting::<AppearanceSettings>(|s| s.light_theme = Some("Custom".into()))
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cx.design(), &expected));
    assert_eq!(notifications.get(), 2);
}

#[gpui_kit::test]
async fn a_superseded_reload_cannot_publish_the_previous_catalogue(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let directories = |name: &str| ThemeDirs {
        user: root.path().join(name),
        installed: root.path().join("installed"),
    };
    let old_dirs = directories("old");
    let new_dirs = directories("new");
    for (dirs, name) in [(&old_dirs, "Old"), (&new_dirs, "New")] {
        std::fs::create_dir(&dirs.user).unwrap();
        std::fs::write(
            dirs.user.join("theme.json"),
            format!(r#"{{"themes":[{{"name":"{name}","appearance":"dark","style":{{}}}}]}}"#),
        )
        .unwrap();
    }
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(
            DesignTokens::builtin(),
            SettingsStore::in_memory(SettingsDocument::default()),
            cx,
        );
        init_themes(old_dirs.clone(), ThemeCatalog::load(&old_dirs), cx);
    });
    let old = cx.update(reload_themes);
    cx.update(|cx| {
        cx.update_global::<Themes, _>(|themes, _| themes.dirs = new_dirs);
    });
    let new = cx.update(reload_themes);
    new.await;
    old.await;
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            cx.global::<Themes>()
                .catalog
                .find("New", Appearance::Dark)
                .is_some()
        );
        assert!(
            cx.global::<Themes>()
                .catalog
                .find("Old", Appearance::Dark)
                .is_none()
        );
    });
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
async fn sparse_imports_composite_over_the_component_background_without_override_leaks(
    cx: &mut TestAppContext,
) {
    use crate::terminal_style::color;
    use gpui_kit::component::ThemeColor;
    use nocterm_design::Color;
    use serde_json::json;

    let directory = tempfile::tempdir().unwrap();
    let dirs = ThemeDirs {
        user: directory.path().join("user"),
        installed: directory.path().join("installed"),
    };
    std::fs::create_dir(&dirs.user).unwrap();
    let mut themes = Vec::new();
    for appearance in ["light", "dark"] {
        for source in ["sparse", "interface", "terminal"] {
            let mut style = json!({
                "terminal.foreground": "#ff000080",
                "terminal.ansi.red": "#00ff0080"
            });
            if source == "interface" {
                style["editor.background"] = json!("#0000ff80");
            } else if source == "terminal" {
                style["terminal.background"] = json!("#0000ff80");
            }
            themes.push(json!({
                "name": format!("{appearance} {source}"),
                "appearance": appearance,
                "style": style
            }));
        }
    }
    std::fs::write(
        dirs.user.join("sparse.json"),
        serde_json::to_vec(&json!({"themes": themes})).unwrap(),
    )
    .unwrap();
    let base = DesignTokens::with_overrides(
        "[light.ui]\nbackground = '#abcdef'\n[dark.ui]\nbackground = '#fedcba'\n\
         [light.terminal]\nbackground = '#123456'\nblue = '#112233'\n\
         [dark.terminal]\nbackground = '#654321'\nblue = '#332211'\n",
    )
    .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(
            base,
            SettingsStore::in_memory(SettingsDocument::default()),
            cx,
        );
        init_themes(dirs.clone(), ThemeCatalog::load(&dirs), cx);
    });
    for dark in [false, true] {
        for source in ["sparse", "interface", "terminal"] {
            let name = format!("{} {source}", if dark { "dark" } else { "light" });
            cx.update(|cx| {
                cx.update_setting::<AppearanceSettings>(move |s| {
                    s.mode = if dark {
                        AppearanceMode::Dark
                    } else {
                        AppearanceMode::Light
                    };
                    if dark {
                        s.dark_theme = Some(name);
                    } else {
                        s.light_theme = Some(name);
                    }
                })
            })
            .await
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                let component = if dark {
                    ThemeColor::dark()
                } else {
                    ThemeColor::light()
                };
                let default = component.background;
                let blue: Color = "#0000ff80".parse().unwrap();
                let background = if source == "sparse" {
                    color(default)
                } else {
                    color(default.blend(hsla(blue)))
                };
                let expected_ui = if source == "interface" {
                    hsla(blue)
                } else {
                    default
                };
                assert_eq!(
                    Theme::global(cx).background,
                    expected_ui,
                    "{dark}/{source} GUI"
                );
                let style = TerminalStyle::current(cx);
                assert_eq!(style.background, background, "{dark}/{source} terminal");
                assert_eq!(
                    style.foreground,
                    color(hsla(background).blend(hsla("#ff000080".parse().unwrap()))),
                    "{dark}/{source} foreground"
                );
                assert_eq!(
                    style.ansi[1],
                    color(hsla(background).blend(hsla("#00ff0080".parse().unwrap()))),
                    "{dark}/{source} ANSI"
                );
                assert_eq!(
                    style.ansi[4],
                    DesignTokens::builtin().palette(dark).terminal.blue,
                    "theme.toml ANSI must not leak"
                );
                assert_eq!(style.background.a, 255);
                assert_eq!(style.foreground.a, 255);
                assert_eq!(style.ansi[1].a, 255);
                if source == "sparse" {
                    assert_eq!(cx.design().palette(dark).terminal.background, None);
                }
            });
        }
    }
}
