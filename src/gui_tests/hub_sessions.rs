//! The sessions of a nocterm hub live in the ordinary window: the panel is offered beside the
//! local features, and a session opens as a terminal tab.
use super::*;
use crate::bootstrap::Booted;
use crate::shell_setup::UserPluginMode;
use nocterm_hubs_model::{
    Fingerprint, HubAddress, HubId, HubRecord, HubsSettings, TrustPolicy,
    testing::{FakeHub, FakeServer},
};

const ADDRESS: &str = "hub.home:7443";

fn hub_id() -> HubId {
    HubId::from(HubAddress::parse(ADDRESS).unwrap())
}

/// A server with one user, signed in to, whose certificate the settings pin.
fn hub() -> FakeHub {
    let hub = FakeHub::new().with_server(
        FakeServer::new(ADDRESS, 7)
            .named("Home hub")
            .with_user("ada", "secret", "lab")
            .with_node("n1", "build box", true)
            .with_session("s1", "lab", "n1", "ada", "make"),
    );
    hub.sign_in_as(&hub_id(), "ada");
    hub
}

fn known_server() -> HubRecord {
    HubRecord::new(
        HubAddress::parse(ADDRESS).unwrap(),
        TrustPolicy::Pinned(Fingerprint::from_bytes([7; 32])),
        "Home",
    )
}

/// The application composed over `hub`, or without hubs at all.
fn boot_over(hub: Option<&FakeHub>, cx: &mut TestAppContext) -> Booted {
    let servers = if hub.is_some() {
        vec![known_server()]
    } else {
        Vec::new()
    };
    let settings =
        nocterm_store::SettingsConfig::in_memory(Default::default()).with(HubsSettings {
            servers,
            ..Default::default()
        });
    let booted = cx.update(|cx| {
        let deps = hub.map(|hub| nocterm_hubs::Deps {
            trust: Arc::new(hub.clone()),
            sign_in: Arc::new(hub.clone()),
            sessions: Arc::new(hub.clone()),
            admin_keys: Arc::new(nocterm_hubs_model::testing::FakeAdminKeys::alongside(&hub)),
        });
        boot_with_ports_and_probe(
            tempfile::tempdir().unwrap(),
            settings,
            (
                Arc::new(MockTransport::default()),
                nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
            ),
            false,
            None,
            None,
            unsupported_sandbox(),
            UserPluginMode::default(),
            deps,
            cx,
        )
    });
    // The servers are contacted in the background.
    cx.run_until_parked();
    booted
}

/// Opens a window the way the application composes one and returns it.
pub(super) fn open(booted: &Booted, cx: &mut TestAppContext) -> AnyWindowHandle {
    let (handle, _) = cx.update(|cx| open_workspace(booted, cx));
    cx.run_until_parked();
    show(handle, cx);
    handle
}

pub(super) fn show(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

pub(super) fn workspace_of(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Entity<Workspace> {
    cx.update_window(handle, |_, window, cx| {
        gpui_kit::base::Root::read(window, cx)
            .view()
            .clone()
            .downcast::<Workspace>()
            .unwrap()
    })
    .unwrap()
}

pub(super) fn dispatch(
    handle: AnyWindowHandle,
    action: impl gpui_kit::Action,
    cx: &mut TestAppContext,
) {
    let workspace = workspace_of(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.focus(&workspace.read(cx).focus_handle(cx), cx);
        window.dispatch_action(Box::new(action), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    show(handle, cx);
}

pub(super) fn click(
    handle: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
    show(handle, cx);
}

/// Waits for the entrance animation of a dialog, so its buttons can be clicked.
pub(super) fn settle_dialog(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    show(handle, cx);
    std::thread::sleep(
        *gpui_kit::component::dialog::ANIMATION_DURATION + std::time::Duration::from_millis(50),
    );
    show(handle, cx);
}

fn shown(handle: AnyWindowHandle, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(handle, |_, window, _| window.try_find(id).is_some())
        .unwrap()
}

fn registered(handle: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    let workspace = workspace_of(handle, cx);
    cx.update(|cx| {
        workspace
            .read(cx)
            .is_registered(&nocterm_hubs_ui::plugin::SESSIONS.id)
    })
}

fn tab_count(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    let workspace = workspace_of(handle, cx);
    cx.update(|cx| workspace.read(cx).items(cx).count())
}

#[gpui_kit::test]
fn the_window_offers_the_sessions_panel_beside_its_local_features(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), cx);
    let window = open(&booted, cx);
    let workspace = workspace_of(window, cx);

    assert!(registered(window, cx));
    assert!(cx.update(|cx| {
        workspace
            .read(cx)
            .is_registered(&nocterm_connections::plugin::SERVERS.id)
    }));
    assert!(
        !cx.update(|cx| {
            workspace
                .read(cx)
                .panel_is_open(&nocterm_hubs_ui::plugin::SESSIONS.id)
        }),
        "the panel waits in the tray until it is asked for"
    );
}

#[gpui_kit::test]
fn an_application_without_hubs_registers_no_sessions_panel(cx: &mut TestAppContext) {
    let booted = boot_over(None, cx);
    let window = open(&booted, cx);

    assert!(!registered(window, cx));
}

#[gpui_kit::test]
fn the_sessions_of_the_hub_open_as_tabs_of_the_window(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), cx);
    let window = open(&booted, cx);

    dispatch(window, nocterm_hubs_ui::OpenHubSessions, cx);
    assert!(shown(window, "hub-session-s1", cx));
    assert_eq!(tab_count(window, cx), 0);

    click(window, "hub-session-open-s1", cx);
    assert_eq!(tab_count(window, cx), 1, "a tab of the session opened");
}
