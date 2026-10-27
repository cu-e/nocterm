//! Fixtures the tests of this crate share: a window over a [`FakeHub`], and a recording
//! stand-in for the port the application implements.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, ElementId, Entity, Focusable as _, SharedString,
    TestAppContext, Window, WindowOptions, component::WindowExt as _, test::TestWindowExt as _,
};
use nocterm_hubs_model::testing::{FakeHub, FakeServer};
use nocterm_hubs_model::{HubAddress, HubId, HubRecord, HubsSettings};
use nocterm_session::{Target, Transport};
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;
use nocterm_ui::DesignTokens;
use nocterm_workspace::{Workspace, test_support::deps_for};

use crate::{HubTabError, HubTerminalOpener, Installed, OpenHubSessions, install, plugin, register};

pub(crate) const ADDRESS: &str = "hub.home:7443";

pub(crate) fn hub_id() -> HubId {
    HubId::from(HubAddress::parse(ADDRESS).unwrap())
}

/// The server most tests talk to: certificate `[7; 32]`, user `ada` in organization `lab`, one
/// machine `build box` with a session of `ada` and one of `bob`.
pub(crate) fn server() -> FakeServer {
    FakeServer::new(ADDRESS, 7)
        .named("Home hub")
        .with_user("ada", "secret", "lab")
        .with_provider("corp", "Corporate SSO")
        .with_node("n1", "build box", true)
        .with_node("n2", "spare box", false)
        .with_session("s1", "lab", "n1", "ada", "make")
        .with_session("s2", "lab", "n1", "bob", "top")
}

/// The record of [`server`] as the settings keep it after a sign-in.
pub(crate) fn pinned_record() -> HubRecord {
    server().pinned_record("Home")
}

/// Remembers the tabs the views asked for.
#[derive(Default)]
pub(crate) struct RecordingTerminals {
    tabs: RefCell<Vec<(SharedString, Target)>>,
}

impl RecordingTerminals {
    pub(crate) fn tabs(&self) -> Vec<(SharedString, Target)> {
        self.tabs.borrow().clone()
    }
}

impl HubTerminalOpener for RecordingTerminals {
    fn open(
        &self,
        _: &mut Workspace,
        title: SharedString,
        target: Target,
        _: Arc<dyn Transport>,
        _: &mut Window,
        _: &mut Context<Workspace>,
    ) -> Result<(), HubTabError> {
        self.tabs.borrow_mut().push((title, target));
        Ok(())
    }
}

pub(crate) struct Fixture {
    pub(crate) window: AnyWindowHandle,
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) hub: FakeHub,
    pub(crate) installed: Installed,
    pub(crate) terminals: Rc<RecordingTerminals>,
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
    let terminals = Rc::new(RecordingTerminals::default());
    let (window, workspace, installed) = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = config.with(HubsSettings { servers: known }).install(cx);
        nocterm_ui::init(DesignTokens::builtin(), settings.clone(), cx);
        let installed = install(nocterm_hubs::install(
            nocterm_hubs::Deps {
                trust: Arc::new(hub.clone()),
                sign_in: Arc::new(hub.clone()),
                sessions: Arc::new(hub.clone()),
            },
            settings,
            cx,
        ));
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(deps_for(&[&plugin::PLUGIN], cx), window, cx))
            })
            .unwrap();
        (window, workspace, installed)
    });
    cx.run_until_parked();
    let setup = Fixture {
        window,
        workspace,
        hub,
        installed,
        terminals,
    };
    setup.compose(cx);
    cx.run_until_parked();
    setup
}

impl Fixture {
    fn compose(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| {
            self.workspace.update(cx, |workspace, cx| {
                register(
                    &self.installed,
                    self.terminals.clone(),
                    workspace,
                    window,
                    cx,
                )
                .unwrap();
            });
        })
        .unwrap();
    }

    /// Opens the sessions panel, which waits in the tray until it is asked for.
    pub(crate) fn open_sessions(&self, cx: &mut TestAppContext) {
        self.dispatch(OpenHubSessions, cx);
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
