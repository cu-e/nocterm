use super::*;
use crate::{ActiveDesign, TerminalStyle, edit_settings, hsla, init};
use gpui_kit::{TestAppContext, component::Theme};
use nocterm_settings::{AppearanceMode, Settings};
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
        let mut settings = Settings::default();
        settings.appearance.mode = AppearanceMode::Dark;
        init(base, SettingsStore::in_memory(settings), cx);
        init_themes(dirs.clone(), ThemeCatalog::load(&dirs), cx);
    });
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    cx.update(|cx| {
        let count = notifications.clone();
        cx.observe_global::<Design>(move |_| count.set(count.get() + 1))
            .detach();
    });
    cx.update(|cx| edit_settings(cx, |s| s.appearance.dark_theme = Some("Custom".into())))
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
        cx.update(|cx| edit_settings(cx, move |s| s.appearance.dark_theme = name))
            .await
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.design(), &expected));
    }
    cx.update(|cx| edit_settings(cx, |s| s.appearance.light_theme = Some("Custom".into())))
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
            SettingsStore::in_memory(Settings::default()),
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
