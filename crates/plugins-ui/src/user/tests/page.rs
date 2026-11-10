//! External plugins in the Plugins page: one list with the built-in plugins, one selection.

use super::*;
use crate::{
    PluginsPage,
    tests::{FakeToggles, catalog},
};

/// The page with the built-in plugins of the crate's fixture and one external plugin.
fn open_page(cx: &mut TestAppContext) -> gpui_kit::AnyWindowHandle {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let store = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), store, cx);
        let plugins = Rc::new(FakeUserPlugins {
            rows: RefCell::new(vec![row()]),
            candidate: RefCell::new(Ok(candidate())),
            dev: RefCell::default(),
            calls: RefCell::default(),
            changed: cx.new(|_| ()),
        });
        let switch: Rc<dyn UserPluginsSwitch> = Rc::new(Rc::new(FakeSwitch {
            allowed: Cell::new(true),
            refuse: RefCell::default(),
            calls: RefCell::default(),
            changed: cx.new(|_| ()),
        }));
        let folder: Arc<dyn PluginsFolder> = Arc::new(FakeFolder::default());
        let toggles = Rc::new(FakeToggles::new(cx));
        let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                PluginsPage::new(
                    catalog(),
                    toggles,
                    PluginSettingsDirectory::new(),
                    window,
                    cx,
                )
                .with_user_plugins(
                    UserSession::Loaded(Rc::new(plugins)),
                    switch,
                    folder,
                    cx,
                )
            })
        })
        .unwrap();
        window
    })
}

fn draw(
    window: gpui_kit::AnyWindowHandle,
    cx: &mut TestAppContext,
    check: impl FnOnce(&mut gpui_kit::Window, &mut App),
) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        check(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn external_plugins_share_the_list_and_one_plugin_is_selected_at_a_time(cx: &mut TestAppContext) {
    let window = open_page(cx);
    draw(window, cx, |window, cx| {
        assert!(window.try_find("plugin-row:tools").is_some());
        assert!(window.try_find("user-plugin-row:user-acme-clock").is_some());
        assert!(
            window
                .try_find("user-chip:user-acme-clock:External")
                .is_some()
        );
        assert!(window.try_find("user-plugins-head").is_some());
        assert!(
            window.try_find("plugin-detail").is_some(),
            "the first built-in one"
        );
        window.click("user-plugin-row:user-acme-clock", cx);
    });
    draw(window, cx, |window, cx| {
        assert_eq!(window.find("user-plugin-detail").label(), Some("Clock"));
        assert!(window.try_find("plugin-detail").is_none());
        window.click("plugin-row:tools", cx);
    });
    draw(window, cx, |window, _| {
        assert_eq!(window.find("plugin-detail").label(), Some("Tools"));
        assert!(window.try_find("user-plugin-detail").is_none());
    });
}
