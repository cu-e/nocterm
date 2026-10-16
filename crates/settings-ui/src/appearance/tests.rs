//! Appearance behavior through rendered controls and explicit shared ports.
use crate::SettingsView;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_themes::{Appearance, ExtensionInfo, ThemeLibrary, test_support::InMemoryThemes};
use nocterm_ui::{AppearanceSettings, ThemeUi};
use std::sync::Arc;
fn info(id: &str) -> ExtensionInfo {
    serde_json::from_str(&format!(r#"{{"id":"{id}","name":"{id}","version":"1"}}"#)).unwrap()
}
fn backend() -> Arc<InMemoryThemes> {
    let backend = Arc::new(InMemoryThemes::default());
    let document=br##"{"themes":[{"name":"Fake Dark","appearance":"dark","style":{"background":"#123"}},{"name":"Fake Light","appearance":"light","style":{"background":"#eee"}}]}"##;
    for id in ["fake", "failure"] {
        backend.offer(info(id), document.to_vec());
    }
    backend.fail_install("failure", true);
    backend
}
fn setup(
    cx: &mut TestAppContext,
    backend: Arc<InMemoryThemes>,
) -> (gpui_kit::AnyWindowHandle, Entity<SettingsView>, ThemeUi) {
    cx.update(|cx| {
        crate::tests::init(cx);
        let ready = nocterm_ui::UiReady::installed(cx).unwrap();
        let service = nocterm_theme_service::ThemesHandle::install(
            Some(backend.clone()),
            Some(backend.clone()),
            backend.scan(),
            cx,
        );
        let themes = ThemeUi::install(
            ready.settings().clone(),
            service,
            nocterm_ui::DesignTokens::builtin(),
            cx,
        );
        let supplied = themes.clone();
        let (handle, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)),
                    gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(2000.)),
                ))),
                ..WindowOptions::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    SettingsView::with_pages(
                        Vec::new(),
                        supplied,
                        crate::tests::settings_deps(cx),
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap();
        (handle, view, themes)
    })
}
fn click(handle: gpui_kit::AnyWindowHandle, id: &'static str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn opening_is_offline_and_browse_requires_an_explicit_action(cx: &mut TestAppContext) {
    let backend = backend();
    let (handle, _, _) = setup(cx, backend.clone());
    cx.run_until_parked();
    assert_eq!(backend.searches(), 0);
    assert_eq!(backend.installs(), 0);
    click(handle, "browse-zed-themes", cx);
    assert_eq!(backend.searches(), 1);
    assert_eq!(backend.installs(), 0);
    click(handle, "install-theme-fake", cx);
    assert_eq!(backend.installs(), 1);
}
#[gpui_kit::test]
async fn uninstall_rebases_current_selection_and_retains_surviving_duplicates(
    cx: &mut TestAppContext,
) {
    let backend = backend();
    backend.install(&info("fake")).unwrap();
    backend.offer(
        info("other"),
        br#"{"themes":[{"name":"Fake Dark","appearance":"dark","style":{}}]}"#.to_vec(),
    );
    backend.install(&info("other")).unwrap();
    let (handle, _, themes) = setup(cx, backend);
    cx.update(|cx| {
        themes.settings().update::<AppearanceSettings>(cx, |s| {
            s.dark_theme = Some("Fake Dark".into());
            s.light_theme = Some("Fake Light".into());
        })
    })
    .await
    .unwrap();
    click(handle, "uninstall-theme-fake", cx);
    cx.update(|cx| {
        let settings = themes.settings().setting::<AppearanceSettings>(cx);
        assert_eq!(settings.dark_theme.as_deref(), Some("Fake Dark"));
        assert_eq!(settings.light_theme, None);
        assert!(
            themes
                .themes()
                .catalog(cx)
                .find("Fake Dark", Appearance::Dark)
                .is_some()
        );
    });
}
#[gpui_kit::test]
fn shared_status_and_admitted_install_survive_the_originating_page(cx: &mut TestAppContext) {
    let backend = backend();
    let (handle, view, themes) = setup(cx, backend.clone());
    let task = cx.update(|cx| themes.themes().install_pack(info("fake"), cx));
    cx.update(|cx| assert!(themes.themes().busy("fake", cx)));
    drop(task);
    drop(view);
    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!themes.themes().busy("fake", cx));
        assert!(
            themes
                .themes()
                .catalog(cx)
                .find("Fake Dark", Appearance::Dark)
                .is_some()
        );
    });
    assert_eq!(backend.installs(), 1);
}
#[gpui_kit::test]
fn plain_settings_without_ports_offer_default_and_do_not_fetch(cx: &mut TestAppContext) {
    let (_, view) = crate::tests::open(cx);
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).selected_page_id(), "appearance"));
}
