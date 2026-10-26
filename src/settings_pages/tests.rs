//! App catalogs validate before factories and keep ordinary/startup policy distinct.
use super::*;
use gpui_kit::{
    App, AppContext as _, Context, FocusHandle, Focusable, Render, TestAppContext, Window, div,
    prelude::*, test::TestWindowExt as _,
};
use nocterm_settings_ui::SettingsView;
use std::{cell::Cell, rc::Rc};
struct Page {
    focus: FocusHandle,
}
impl Focusable for Page {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Page {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl nocterm_ui::SettingsPage for Page {}
fn spec(id: &'static str, called: Rc<Cell<usize>>) -> SettingsPageSpec {
    SettingsPageSpec::new(id, id, move |_, cx| {
        called.set(called.get() + 1);
        cx.new(|cx| Page {
            focus: cx.focus_handle(),
        })
    })
}
fn guests(specs: Vec<SettingsPageSpec>) -> GuestPages {
    let plugins = Arc::new(crate::shell_setup::plugin_catalog().unwrap());
    GuestPages::new(
        specs
            .into_iter()
            .map(SettingsPageRegistration::lazy)
            .collect(),
        plugins,
    )
}
fn builtins(called: Rc<Cell<usize>>) -> [SettingsPageSpec; 5] {
    ["appearance", "terminal", "local-shell", "ssh", "ai"].map(|id| spec(id, called.clone()))
}
#[gpui_kit::test]
fn actual_root_catalog_builder_propagates_typed_registration_errors_without_factory_calls(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let installed = Installed::new(settings);
        let called = Rc::new(Cell::new(0));
        let duplicate = SettingsPages::new(
            installed.clone(),
            builtins(called.clone()),
            guests(vec![spec("terminal", called.clone())]),
            None,
        );
        assert_eq!(
            duplicate.err(),
            Some(SettingsCatalogError::DuplicateId { id: "terminal" })
        );
        let mut invalid = builtins(called.clone());
        invalid[0] = spec("other", called.clone());
        assert_eq!(
            SettingsPages::new(installed, invalid, guests(vec![]), None).err(),
            Some(SettingsCatalogError::MissingDefault { id: "appearance" })
        );
        assert_eq!(called.get(), 0);
    });
}
#[gpui_kit::test]
fn ordinary_and_startup_catalogs_keep_their_distinct_guests_and_per_item_instances(
    cx: &mut TestAppContext,
) {
    let called = Rc::new(Cell::new(0));
    let (ordinary_window, ordinary, startup_window, startup) = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), settings.clone(), cx);
        let installed = Installed::new(settings);
        let pages = SettingsPages::new(
            installed.clone(),
            builtins(called.clone()),
            guests(
                ["keymap", "monitor", "explorer", "workspaces"]
                    .into_iter()
                    .map(|id| spec(id, called.clone()))
                    .collect(),
            ),
            Some(spec("vault", called.clone())),
        )
        .unwrap();
        let open = |catalog, cx: &mut App| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| SettingsView::with_pages(catalog, installed.clone(), window, cx))
            })
            .unwrap()
        };
        let (ordinary_window, ordinary) = open(pages.ordinary().clone(), cx);
        let (startup_window, startup) = open(pages.startup().unwrap().clone(), cx);
        (ordinary_window, ordinary, startup_window, startup)
    });
    assert_eq!(
        called.get(),
        10,
        "five eager independent page instances per Item"
    );
    cx.update_window(ordinary_window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(ordinary.read(cx).selected_page_id(), "appearance");
        assert!(window.within("settings-pages").try_find(9usize).is_some());
        assert!(window.within("settings-pages").try_find(10usize).is_none());
    })
    .unwrap();
    cx.update_window(startup_window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.within("settings-pages").try_find(6usize).is_none());
        window.within("settings-pages").click(5usize, cx);
        assert_eq!(startup.read(cx).selected_page_id(), "vault");
        assert_eq!(called.get(), 11);
    })
    .unwrap();
}

#[gpui_kit::test]
fn only_the_ordinary_appearance_hosts_the_sections_the_application_supplies(
    cx: &mut TestAppContext,
) {
    let called = Rc::new(Cell::new(0));
    let (ordinary_window, startup_window) = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), settings.clone(), cx);
        let installed = Installed::new(settings);
        let section =
            nocterm_ui::EmbeddedPageSpec::new("layout", "Status bar and panels", |_, cx| {
                cx.new(|cx| Page {
                    focus: cx.focus_handle(),
                })
            });
        let pages = SettingsPages::new(
            installed.clone(),
            builtins(called.clone()),
            guests(vec![]).with_appearance_sections(vec![section]),
            Some(spec("vault", called.clone())),
        )
        .unwrap();
        let open = |catalog, cx: &mut App| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| SettingsView::with_pages(catalog, installed.clone(), window, cx))
            })
            .unwrap()
            .0
        };
        (
            open(pages.ordinary().clone(), cx),
            open(pages.startup().unwrap().clone(), cx),
        )
    });
    cx.update_window(ordinary_window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-section:layout").is_some());
    })
    .unwrap();
    cx.update_window(startup_window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-section:layout").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn catalog_keeps_the_real_keymap_page_alive_without_a_booted_fixture_handle(
    cx: &mut TestAppContext,
) {
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), settings.clone(), cx);
        let installed = Installed::new(settings);
        let pages = {
            let keymap = crate::keymap::load(None, cx);
            SettingsPages::new(
                installed.clone(),
                builtins(Rc::new(Cell::new(0))),
                guests(vec![nocterm_keymap_ui::settings_page(keymap)]),
                None,
            )
            .unwrap()
        };
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| SettingsView::with_pages(pages.ordinary().clone(), installed, window, cx))
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("settings-pages").click(5usize, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_page_id(), "keymap");
        assert!(window.try_find("keymap-page").is_some());
        assert!(
            window.try_find(("keymap-row", 0usize)).is_some(),
            "the actual retained keymap supplies commands after composition inputs leave scope"
        );
    })
    .unwrap();
}
