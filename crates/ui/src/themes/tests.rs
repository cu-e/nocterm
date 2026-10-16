use super::*;
use crate::{ActiveDesign, TerminalStyle, hsla, init};
use crate::{AppearanceMode, AppearanceSettings};
use gpui_kit::{TestAppContext, component::Theme};
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;
use nocterm_themes::ThemeLibrary;
fn install_test_projection(
    backend: std::sync::Arc<nocterm_themes::test_support::InMemoryThemes>,
    base: DesignTokens,
    settings: SettingsConfig,
    cx: &mut App,
) -> ThemeUi {
    gpui_kit::init(cx);
    let ready = init(base.clone(), settings.install(cx), cx);
    let service = nocterm_theme_service::ThemesHandle::install(
        Some(backend.clone()),
        None,
        backend.scan(),
        cx,
    );
    ThemeUi::install(ready.settings().clone(), service, base, cx)
}

#[gpui_kit::test]
async fn selection_applies_ui_and_terminal_then_clears_without_a_loop(cx: &mut TestAppContext) {
    let backend = std::sync::Arc::new(nocterm_themes::test_support::InMemoryThemes::default());
    backend.user_document("test.json".into(),br##"{"themes":[{"name":"Custom","appearance":"dark","style":{"editor.background":"#112233","text":"#eeeeee","terminal.ansi.red":"#ff0000"}}]}"##.to_vec());
    let base = DesignTokens::with_overrides("[dark.terminal]\nblue = '#abcdef'\n").unwrap();
    let expected = base.clone();
    let _themes = cx.update(|cx| {
        let appearance = AppearanceSettings {
            mode: AppearanceMode::Dark,
            ..AppearanceSettings::default()
        };
        let settings = SettingsConfig::in_memory(SettingsDocument::default()).with(appearance);
        install_test_projection(backend.clone(), base, settings, cx)
    });
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    cx.update(|cx| {
        let count = notifications.clone();
        cx.observe_global::<Design>(move |_| count.set(count.get() + 1))
            .detach();
    });
    cx.update(|cx| {
        _themes
            .settings()
            .update::<AppearanceSettings>(cx, |s| s.dark_theme = Some("Custom".into()))
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
            TerminalStyle::with_settings(
                _themes.settings().setting::<crate::TerminalSettings>(cx),
                cx
            )
            .background,
            "#112233".parse().unwrap()
        );
        assert_eq!(
            cx.design().dark.terminal.normal.red,
            "#ff0000".parse().unwrap()
        );
        assert_eq!(
            cx.design().dark.terminal.normal.blue,
            DesignTokens::builtin().dark.terminal.normal.blue
        );
        assert!(cx.global::<Design>().is_imported(true));
        assert!(!cx.global::<Design>().is_imported(false));
    });
    assert_eq!(notifications.get(), 1);
    for name in [Some("Missing".to_owned()), None] {
        cx.update(|cx| {
            _themes
                .settings()
                .update::<AppearanceSettings>(cx, move |s| s.dark_theme = name)
        })
        .await
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.design(), &expected));
    }
    cx.update(|cx| {
        _themes
            .settings()
            .update::<AppearanceSettings>(cx, |s| s.light_theme = Some("Custom".into()))
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cx.design(), &expected));
    assert_eq!(notifications.get(), 2);
}

#[gpui_kit::test]
async fn sparse_imports_composite_over_the_component_background_without_override_leaks(
    cx: &mut TestAppContext,
) {
    use serde_json::json;

    let backend = std::sync::Arc::new(nocterm_themes::test_support::InMemoryThemes::default());
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
    backend.user_document(
        "sparse.json".into(),
        serde_json::to_vec(&json!({"themes":themes})).unwrap(),
    );
    let base = DesignTokens::with_overrides(
        "[light.ui]\nbackground = '#abcdef'\n[dark.ui]\nbackground = '#fedcba'\n\
         [light.terminal]\nbackground = '#123456'\nblue = '#112233'\n\
         [dark.terminal]\nbackground = '#654321'\nblue = '#332211'\n",
    )
    .unwrap();
    let _themes = cx.update(|cx| {
        install_test_projection(
            backend.clone(),
            base,
            SettingsConfig::in_memory(SettingsDocument::default()),
            cx,
        )
    });
    for dark in [false, true] {
        for source in ["sparse", "interface", "terminal"] {
            let name = format!("{} {source}", if dark { "dark" } else { "light" });
            cx.update(|cx| select_variant(&_themes, dark, name, cx))
                .await
                .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                verify_sparse_palette(
                    dark,
                    source,
                    _themes.settings().setting::<crate::TerminalSettings>(cx),
                    cx,
                )
            });
        }
    }
}

fn select_variant(
    projection: &ThemeUi,
    dark: bool,
    name: String,
    cx: &mut App,
) -> gpui_kit::Task<Result<u64, crate::SettingsError>> {
    projection
        .settings()
        .update::<AppearanceSettings>(cx, move |s| {
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
}

fn verify_sparse_palette(
    dark: bool,
    source: &str,
    settings: &crate::TerminalSettings,
    cx: &gpui_kit::App,
) {
    use crate::terminal_style::color;
    use gpui_kit::component::ThemeColor;
    use nocterm_design::Color;
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
    let style = TerminalStyle::with_settings(settings, cx);
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
        DesignTokens::builtin().palette(dark).terminal.normal.blue,
        "theme.toml ANSI must not leak"
    );
    assert_eq!(style.background.a, 255);
    assert_eq!(style.foreground.a, 255);
    assert_eq!(style.ansi[1].a, 255);
    if source == "sparse" {
        assert_eq!(cx.design().palette(dark).terminal.background, None);
    }
}

mod projection_authority;
