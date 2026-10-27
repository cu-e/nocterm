//! A window shows one environment: this computer, or one server of nocterm hubs.
use super::*;
use crate::bootstrap::Booted;
use crate::shell_setup::UserPluginMode;
use gpui_kit::VisualTestContext;
use nocterm_hubs_model::{
    Environment, Fingerprint, HubAddress, HubId, HubRecord, HubsSettings, TrustPolicy,
    testing::{FakeHub, FakeServer},
};
use nocterm_session::Command;
use nocterm_workspace::NewWindow;

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
fn boot_over(
    hub: Option<&FakeHub>,
    transport: &Arc<MockTransport>,
    cx: &mut TestAppContext,
) -> Booted {
    let booted = cx.update(|cx| {
        let (servers, deps) = match hub {
            Some(hub) => (
                vec![known_server()],
                Some(nocterm_hubs::Deps {
                    trust: Arc::new(hub.clone()),
                    sign_in: Arc::new(hub.clone()),
                    sessions: Arc::new(hub.clone()),
                }),
            ),
            None => (Vec::new(), None),
        };
        boot_with_ports_and_probe(
            tempfile::tempdir().unwrap(),
            nocterm_store::SettingsConfig::in_memory(Default::default())
                .with(HubsSettings { servers }),
            (
                transport.clone(),
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

/// Opens a window the way the application does and returns it.
pub(super) fn open(
    booted: &Booted,
    environment: Environment,
    cx: &mut TestAppContext,
) -> AnyWindowHandle {
    let before = cx.update(|cx| cx.windows());
    cx.update(|cx| crate::windows::open_window(cx, booted, environment))
        .unwrap();
    cx.run_until_parked();
    let opened = cx
        .update(|cx| cx.windows())
        .into_iter()
        .find(|window| !before.contains(window))
        .expect("a new window");
    show(opened, cx);
    opened
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

pub(super) fn title_of(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Option<String> {
    VisualTestContext::from_window(handle, cx).window_title()
}

fn registered(
    handle: AnyWindowHandle,
    id: &nocterm_plugin_model::ContributionId,
    cx: &mut TestAppContext,
) -> bool {
    let workspace = workspace_of(handle, cx);
    cx.update(|cx| workspace.read(cx).is_registered(id))
}

fn tab_count(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    let workspace = workspace_of(handle, cx);
    cx.update(|cx| workspace.read(cx).items(cx).count())
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

fn windows(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

/// The row of the known server in the environment menu: this computer comes first.
const SERVER_ROW: usize = 1;

#[gpui_kit::test]
fn a_hub_window_has_the_sessions_panel_and_none_of_the_local_features(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Hub(hub_id()), cx);

    assert!(registered(
        window,
        &nocterm_hubs_ui::plugin::SESSIONS.id,
        cx
    ));
    for local in [
        &nocterm_connections::plugin::SERVERS.id,
        &nocterm_files::plugin::EXPLORER.id,
        &nocterm_files::plugin::TRANSFERS.id,
        &nocterm_containers_ui::plugin::LIST.id,
        &nocterm_projects::plugin::WORKSPACES.id,
        &nocterm_monitor_ui::plugin::SUMMARY.id,
        &nocterm_agent::plugin::CHATS.id,
    ] {
        assert!(!registered(window, local, cx), "{local:?} is local only");
    }
}

#[gpui_kit::test]
fn a_hub_window_shows_its_sessions_before_the_snippets(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Hub(hub_id()), cx);
    let workspace = workspace_of(window, cx);

    assert!(cx.update(|cx| {
        workspace
            .read(cx)
            .panel_is_open(&nocterm_hubs_ui::plugin::SESSIONS.id)
    }));
}

#[gpui_kit::test]
fn a_local_window_keeps_its_features_and_gains_the_environment_menu(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Local, cx);

    assert!(registered(
        window,
        &nocterm_connections::plugin::SERVERS.id,
        cx
    ));
    assert!(registered(window, &nocterm_files::plugin::EXPLORER.id, cx));
    assert!(!registered(
        window,
        &nocterm_hubs_ui::plugin::SESSIONS.id,
        cx
    ));
    assert!(
        cx.update_window(window, |_, window, _| window
            .try_find("environment-menu")
            .is_some())
            .unwrap()
    );
    assert_eq!(title_of(window, cx).as_deref(), Some("Nocterm"));
}

#[gpui_kit::test]
fn a_hub_window_opens_no_local_tab(cx: &mut TestAppContext) {
    let transport = Arc::new(MockTransport::default());
    let booted = boot_over(Some(&hub()), &transport, cx);
    let window = open(&booted, Environment::Hub(hub_id()), cx);
    let workspace = workspace_of(window, cx);

    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_session(spec("local.test"), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(tab_count(window, cx), 0);
    assert!(transport.drivers.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn a_hub_window_is_titled_after_its_server(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Hub(hub_id()), cx);

    assert_eq!(
        title_of(window, cx).as_deref(),
        Some("Nocterm \u{2014} Home")
    );
}

#[gpui_kit::test]
fn a_server_that_is_not_in_the_list_opens_no_window(cx: &mut TestAppContext) {
    let booted = boot_over(None, &Arc::new(MockTransport::default()), cx);
    let before = windows(cx);

    let refused =
        cx.update(|cx| crate::windows::open_window(cx, &booted, Environment::Hub(hub_id())));

    assert!(refused.is_err());
    assert_eq!(windows(cx), before);
}

#[gpui_kit::test]
fn switching_an_empty_window_replaces_its_workspace_and_keeps_the_window(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Local, cx);
    let old = workspace_of(window, cx).downgrade();
    let windows_before = windows(cx);

    dispatch(window, nocterm_hubs_ui::OpenEnvironmentMenu, cx);
    click(window, ("environment-row", SERVER_ROW), cx);

    assert_eq!(windows(cx), windows_before, "the window stays");
    assert!(registered(
        window,
        &nocterm_hubs_ui::plugin::SESSIONS.id,
        cx
    ));
    assert!(!registered(
        window,
        &nocterm_connections::plugin::SERVERS.id,
        cx
    ));
    assert_eq!(
        title_of(window, cx).as_deref(),
        Some("Nocterm \u{2014} Home")
    );
    assert!(
        old.upgrade().is_none(),
        "nothing keeps the old workspace alive"
    );
}

#[gpui_kit::test]
fn a_hub_window_switches_back_to_this_computer(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Hub(hub_id()), cx);

    dispatch(window, nocterm_hubs_ui::OpenEnvironmentMenu, cx);
    click(window, ("environment-row", 0usize), cx);

    assert!(registered(
        window,
        &nocterm_connections::plugin::SERVERS.id,
        cx
    ));
    assert!(!registered(
        window,
        &nocterm_hubs_ui::plugin::SESSIONS.id,
        cx
    ));
    assert_eq!(title_of(window, cx).as_deref(), Some("Nocterm"));
}

#[gpui_kit::test]
fn switching_a_window_with_a_local_tab_closes_its_session(cx: &mut TestAppContext) {
    let transport = Arc::new(MockTransport::default());
    let booted = boot_over(Some(&hub()), &transport, cx);
    let window = open(&booted, Environment::Local, cx);
    let workspace = workspace_of(window, cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_session(spec("first.test"), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let driver = transport.drivers.lock().unwrap()[0].clone();
    let closed = cx.background_executor.spawn(async move {
        while let Some(command) = driver.next_command().await {
            if matches!(command, Command::Close) {
                return true;
            }
        }
        false
    });
    drop(workspace);

    dispatch(window, nocterm_hubs_ui::OpenEnvironmentMenu, cx);
    click(window, ("environment-row", SERVER_ROW), cx);
    settle_dialog(window, cx);
    click(window, "switch-here", cx);

    assert!(registered(
        window,
        &nocterm_hubs_ui::plugin::SESSIONS.id,
        cx
    ));
    assert_eq!(
        closed.now_or_never(),
        Some(true),
        "the session of the closed tab receives Close"
    );
}

#[gpui_kit::test]
fn cancelling_the_question_leaves_the_tab_and_the_window_as_they_were(cx: &mut TestAppContext) {
    let transport = Arc::new(MockTransport::default());
    let booted = boot_over(Some(&hub()), &transport, cx);
    let window = open(&booted, Environment::Local, cx);
    let workspace = workspace_of(window, cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_session(spec("first.test"), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    dispatch(window, nocterm_hubs_ui::OpenEnvironmentMenu, cx);
    click(window, ("environment-row", SERVER_ROW), cx);
    settle_dialog(window, cx);
    click(window, "switch-cancel", cx);

    assert_eq!(windows(cx), 1);
    assert_eq!(tab_count(window, cx), 1);
    assert!(registered(
        window,
        &nocterm_connections::plugin::SERVERS.id,
        cx
    ));
}

#[gpui_kit::test]
fn a_new_window_for_a_server_leaves_the_local_window_unchanged(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let local = open(&booted, Environment::Local, cx);
    let before = cx.update(|cx| cx.windows());

    dispatch(local, nocterm_hubs_ui::OpenEnvironmentMenu, cx);
    click(local, ("environment-new-window", SERVER_ROW), cx);

    let all = cx.update(|cx| cx.windows());
    assert_eq!(all.len(), 2);
    let opened = all.into_iter().find(|w| !before.contains(w)).unwrap();
    show(opened, cx);
    assert!(registered(
        opened,
        &nocterm_hubs_ui::plugin::SESSIONS.id,
        cx
    ));
    assert_eq!(
        title_of(opened, cx).as_deref(),
        Some("Nocterm \u{2014} Home")
    );
    assert!(registered(
        local,
        &nocterm_connections::plugin::SERVERS.id,
        cx
    ));
    assert!(!registered(
        local,
        &nocterm_hubs_ui::plugin::SESSIONS.id,
        cx
    ));
    assert_eq!(title_of(local, cx).as_deref(), Some("Nocterm"));
}

#[gpui_kit::test]
fn the_new_window_command_of_a_hub_window_opens_this_computer(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let hub_window = open(&booted, Environment::Hub(hub_id()), cx);
    let before = cx.update(|cx| cx.windows());

    dispatch(hub_window, NewWindow, cx);

    let all = cx.update(|cx| cx.windows());
    assert_eq!(all.len(), 2);
    let opened = all.into_iter().find(|w| !before.contains(w)).unwrap();
    show(opened, cx);
    assert!(registered(
        opened,
        &nocterm_connections::plugin::SERVERS.id,
        cx
    ));
}

#[gpui_kit::test]
fn without_hubs_a_window_has_no_environment_menu(cx: &mut TestAppContext) {
    let booted = boot_over(None, &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Local, cx);

    assert!(
        cx.update_window(window, |_, window, _| window
            .try_find("environment-menu")
            .is_none())
            .unwrap()
    );
    assert!(registered(
        window,
        &nocterm_connections::plugin::SERVERS.id,
        cx
    ));
}
