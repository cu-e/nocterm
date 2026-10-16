//! Publication and retained paint ownership through public handles and native windows.
use super::*;
use crate::{TerminalSettings, UiLayout};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, WindowAppearance,
    component::{ActiveTheme as _, button::ButtonMetrics, floating::FloatingCards},
    div,
    prelude::*,
    px,
};
use std::{cell::Cell, rc::Rc};

fn install_projection(cx: &mut gpui_kit::App) -> ThemeUi {
    gpui_kit::init(cx);
    let base = DesignTokens::builtin();
    let ready = init(
        base.clone(),
        SettingsStore::in_memory(Default::default()),
        cx,
    );
    let service = nocterm_theme_service::ThemesHandle::install(None, None, Default::default(), cx);
    ThemeUi::install(ready.settings().clone(), service, base, cx)
}

#[gpui_kit::test]
async fn mode_and_geometry_publish_without_design_changes_or_an_unrelated_document_redirect(
    cx: &mut TestAppContext,
) {
    let projection = cx.update(install_projection);
    let changes = Rc::new(Cell::new(0));
    let _design = cx.update(|cx| {
        let changes = changes.clone();
        cx.observe_global::<Design>(move |_| changes.set(changes.get() + 1))
    });
    let unrelated = cx.update(|cx| {
        let other = nocterm_store::SettingsConfig::in_memory(Default::default())
            .with(AppearanceSettings {
                mode: AppearanceMode::Light,
                ..Default::default()
            })
            .install(cx);
        cx.set_global(SettingsStore::from_handle(other.clone()));
        other
    });
    cx.update(|cx| {
        projection.settings().update::<AppearanceSettings>(cx, |s| {
            s.mode = AppearanceMode::Dark;
            s.layout = UiLayout::Floating;
            s.card_gap = 9.;
            s.card_radius = 11.;
        })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(cx.theme().is_dark());
        let cards = FloatingCards::get(cx).unwrap();
        assert_eq!(cards.gap, px(9.));
        assert_eq!(cards.radius, px(11.));
        assert_eq!(cards.border, cx.theme().border);
        let background =
            TerminalStyle::with_settings(projection.settings().setting::<TerminalSettings>(cx), cx)
                .background;
        assert_eq!(cards.surface, hsla(background));
        assert_eq!(
            cx.global::<ButtonMetrics>().height,
            px(cx.design().layout.button_height * cx.design().typography.ui_size.unwrap_or(16.))
        );
        assert_eq!(
            unrelated.setting::<AppearanceSettings>(cx).mode,
            AppearanceMode::Light
        );
    });
    assert_eq!(changes.get(), 0, "mode and geometry do not replace Design");
    cx.update(|cx| {
        unrelated.update::<AppearanceSettings>(cx, |s| {
            s.mode = AppearanceMode::System;
            s.layout = UiLayout::Classic;
        })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(cx.theme().is_dark()));
    cx.update(|cx| {
        projection.settings().update::<AppearanceSettings>(cx, |s| {
            s.mode = AppearanceMode::Light;
            s.layout = UiLayout::Classic;
        })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!cx.theme().is_dark());
        assert!(FloatingCards::get(cx).is_none());
    });
    assert_eq!(changes.get(), 0);
}

#[gpui_kit::test]
async fn design_notification_paints_without_resolving_and_drop_releases_projection_observers(
    cx: &mut TestAppContext,
) {
    let projection = cx.update(install_projection);
    let settings = projection.settings().clone();
    cx.update(|cx| settings.update::<AppearanceSettings>(cx, |s| s.mode = AppearanceMode::Dark))
        .await
        .unwrap();
    cx.run_until_parked();
    let changes = Rc::new(Cell::new(0));
    let _subscription = cx.update(|cx| {
        let changes = changes.clone();
        cx.observe_global::<Design>(move |_| changes.set(changes.get() + 1))
    });
    let custom = DesignTokens::with_overrides("[dark.ui]\nbackground = '#123456'\n").unwrap();
    cx.update(|cx| cx.set_global(Design::new(custom.clone())));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.design(),
            &custom,
            "paint never resolves over the published Design"
        );
        assert_eq!(cx.theme().background, hsla("#123456".parse().unwrap()));
    });
    assert_eq!(changes.get(), 1, "no Design→resolve feedback loop");
    drop(projection);
    cx.update(|cx| settings.update::<AppearanceSettings>(cx, |s| s.mode = AppearanceMode::Light))
        .await
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            cx.theme().is_dark(),
            "released settings observation cannot repaint"
        );
        cx.set_global(Design::new(DesignTokens::builtin()));
    });
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cx.theme().background, hsla("#123456".parse().unwrap())));
    assert_eq!(changes.get(), 2);
}

struct Surface;
impl gpui_kit::Render for Surface {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("supplied-theme-surface")
            .size_full()
            .bg(cx.theme().background)
    }
}
fn window(projection: &ThemeUi, cx: &mut TestAppContext) -> AnyWindowHandle {
    let projection = projection.clone();
    cx.add_window(move |window, _| {
        window
            .observe_window_appearance(move |_, cx| projection.apply(cx))
            .detach();
        Surface
    })
    .into()
}
#[gpui_kit::test]
async fn retained_projection_serves_surviving_and_new_windows_and_live_forced_mode(
    cx: &mut TestAppContext,
) {
    let projection = cx.update(install_projection);
    let first = window(&projection, cx);
    let second = window(&projection, cx);
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    let third = window(&projection, cx);
    for mode in [
        AppearanceMode::Dark,
        AppearanceMode::System,
        AppearanceMode::Light,
    ] {
        cx.update(|cx| {
            projection
                .settings()
                .update::<AppearanceSettings>(cx, move |s| s.mode = mode)
        })
        .await
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            projection.apply(cx);
            let expected = match mode {
                AppearanceMode::Dark => true,
                AppearanceMode::Light => false,
                AppearanceMode::System => matches!(
                    cx.window_appearance(),
                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                ),
            };
            assert_eq!(cx.theme().is_dark(), expected);
        });
        for handle in [second, third] {
            cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn rejected_native_settings_write_never_paints_the_candidate(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let projection = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = init(
            DesignTokens::builtin(),
            SettingsStore::new(
                Default::default(),
                nocterm_settings::SettingsFile::new(directory.path()),
            ),
            cx,
        );
        let themes =
            nocterm_theme_service::ThemesHandle::install(None, None, Default::default(), cx);
        ThemeUi::install(
            ready.settings().clone(),
            themes,
            DesignTokens::builtin(),
            cx,
        )
    });
    let before = cx.update(|cx| cx.theme().is_dark());
    let receipt = cx.update(|cx| {
        projection.settings().update::<AppearanceSettings>(cx, |s| {
            s.mode = AppearanceMode::Dark;
            s.card_gap = 12.;
        })
    });
    cx.run_until_parked();
    assert!(receipt.await.is_err(), "the destination is a directory");
    cx.update(|cx| {
        assert_eq!(projection.settings().revision(cx), 0);
        assert_eq!(
            projection.settings().setting::<AppearanceSettings>(cx).mode,
            AppearanceMode::System
        );
        assert_eq!(cx.theme().is_dark(), before);
        assert_ne!(FloatingCards::get(cx).unwrap().gap, px(12.));
    });
}
