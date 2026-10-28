//! Fixtures the tests of this crate share: a window over a [`FakeHub`].

use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, Focusable as _, TestAppContext,
    WindowOptions, component::WindowExt as _, test::TestWindowExt as _,
};
use nocterm_hubs_model::testing::{FakeAdminKeys, FakeHub, FakeServer};
use nocterm_hubs_model::{HubAddress, HubId, HubRecord, HubsSettings};
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;
use nocterm_ui::DesignTokens;
use nocterm_workspace::{Workspace, test_support::deps_for};

use crate::{Installed, install, register};

pub(crate) const ADDRESS: &str = "hub.home:7443";

pub(crate) fn hub_id() -> HubId {
    HubId::from(HubAddress::parse(ADDRESS).unwrap())
}

/// The server most tests talk to: certificate `[7; 32]`, user `ada` in organization `lab`.
pub(crate) fn server() -> FakeServer {
    FakeServer::new(ADDRESS, 7)
        .named("Home hub")
        .with_user("ada", "secret", "lab")
        .with_provider("corp", "Corporate SSO")
}

/// The record of [`server`] as the settings keep it after a sign-in.
pub(crate) fn pinned_record() -> HubRecord {
    server().pinned_record("Home")
}

pub(crate) struct Fixture {
    pub(crate) window: AnyWindowHandle,
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) hub: FakeHub,
    pub(crate) admin_keys: FakeAdminKeys,
    pub(crate) installed: Installed,
}

/// A window over `hub`, with `known` servers in the settings. The hub is signed in to when
/// `signed_in` names a user of the server.
pub(crate) fn fixture(
    cx: &mut TestAppContext,
    hub: FakeHub,
    known: Vec<HubRecord>,
    signed_in: Option<&str>,
) -> Fixture {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    fixture_over(cx, config, hub, known, signed_in)
}

/// As [`fixture`], but the settings are kept by `config`, which may fail to save.
pub(crate) fn fixture_over(
    cx: &mut TestAppContext,
    config: SettingsConfig,
    hub: FakeHub,
    known: Vec<HubRecord>,
    signed_in: Option<&str>,
) -> Fixture {
    if let Some(user) = signed_in {
        hub.sign_in_as(&hub_id(), user);
    }
    let admin_keys = FakeAdminKeys::alongside(&hub);
    let (window, workspace, installed) = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = config
            .with(HubsSettings {
                servers: known,
                ..Default::default()
            })
            .install(cx);
        nocterm_ui::init(DesignTokens::builtin(), settings.clone(), cx);
        let installed = install(nocterm_hubs::install(
            nocterm_hubs::Deps {
                trust: Arc::new(hub.clone()),
                sign_in: Arc::new(hub.clone()),
                admin_keys: Arc::new(admin_keys.clone()),
            },
            settings,
            cx,
        ));
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(deps_for(&[], cx), window, cx))
            })
            .unwrap();
        (window, workspace, installed)
    });
    cx.run_until_parked();
    let setup = Fixture {
        window,
        workspace,
        hub,
        admin_keys,
        installed,
    };
    setup.compose(cx);
    cx.run_until_parked();
    setup
}

impl Fixture {
    fn compose(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, _, cx| {
            self.workspace.update(cx, |workspace, _| {
                register(&self.installed, workspace);
            });
        })
        .unwrap();
    }

    /// Draws the window.
    pub(crate) fn render(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    /// Runs `action` as if the focused workspace received it.
    pub(crate) fn dispatch(&self, action: impl gpui_kit::Action, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| {
            window.focus(&self.workspace.read(cx).focus_handle(cx), cx);
            window.dispatch_action(Box::new(action), cx);
        })
        .unwrap();
        self.render(cx);
        self.render(cx);
    }

    pub(crate) fn click(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| window.click(id, cx))
            .unwrap();
        self.render(cx);
    }

    pub(crate) fn press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| window.press(key, cx))
            .unwrap();
        self.render(cx);
    }

    pub(crate) fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| window.input(text, cx))
            .unwrap();
        self.render(cx);
    }

    /// Whether an element with `id` was drawn.
    pub(crate) fn shows(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.window, |_, window, _| window.try_find(id).is_some())
            .unwrap()
    }

    pub(crate) fn has_dialog(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.window, |_, window, cx| window.has_active_dialog(cx))
            .unwrap()
    }

    /// Waits for the entrance animation of a dialog, so its buttons can be clicked.
    pub(crate) fn settle_dialog(&self, cx: &mut TestAppContext) {
        self.render(cx);
        std::thread::sleep(
            *gpui_kit::component::dialog::ANIMATION_DURATION + std::time::Duration::from_millis(50),
        );
        self.render(cx);
    }
}
