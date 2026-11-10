//! A user signs in to a real hub and signs out of it, through the windows.
//!
//! Needs a running hub (`docker compose up` in the nocterm-server repository) and the variables
//! of `crates/hub-io/tests/hub.rs`: `NOCTERM_HUB_ADDRESS`, `NOCTERM_HUB_FINGERPRINT`,
//! `NOCTERM_HUB_USERNAME` and `NOCTERM_HUB_PASSWORD`. Without `NOCTERM_REQUIRE_HUB` the test
//! passes vacuously when they are missing; with it, missing variables or an unreachable hub
//! fail the run.
//!
//! The application is composed as in the other window tests, but over the real `HubLinks` on a
//! temporary credentials directory. The adapter works on its own runtime, so the test waits in
//! real time: it polls the windows with short sleeps until a deadline.
mod account_key;
mod agent_chat;

use super::*;
use crate::bootstrap::Booted;
use crate::shell_setup::UserPluginMode;
use futures::executor::block_on;
use nocterm_hub_io::{HubLinks, HubLinksConfig};
use nocterm_hubs_model::{
    Fingerprint, HubAddress, HubEndpointSpec, HubId, HubSignIn as _, HubTrust as _, HubsSettings,
    TrustPolicy,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SnapshotWriter};
use std::time::{Duration, Instant};

/// How long the hub gets to answer.
const PATIENCE: Duration = Duration::from_secs(45);

struct Hub {
    address: HubAddress,
    fingerprint: Fingerprint,
    username: String,
    password: String,
}

impl Hub {
    fn id(&self) -> HubId {
        HubId::from(self.address.clone())
    }
}

/// The hub the test variables describe, or `None` when the test is not asked to run.
fn hub() -> Option<Hub> {
    let variable = |name: &str| std::env::var(name).ok();
    let settings = (
        variable("NOCTERM_HUB_ADDRESS"),
        variable("NOCTERM_HUB_FINGERPRINT"),
        variable("NOCTERM_HUB_USERNAME"),
        variable("NOCTERM_HUB_PASSWORD"),
    );
    let (Some(address), Some(fingerprint), Some(username), Some(password)) = settings else {
        assert!(
            std::env::var_os("NOCTERM_REQUIRE_HUB").is_none(),
            "NOCTERM_REQUIRE_HUB is set but NOCTERM_HUB_ADDRESS, NOCTERM_HUB_FINGERPRINT, \
             NOCTERM_HUB_USERNAME and NOCTERM_HUB_PASSWORD are not all set"
        );
        return None;
    };
    Some(Hub {
        address: HubAddress::parse(&address).expect("NOCTERM_HUB_ADDRESS"),
        fingerprint: Fingerprint::parse(&fingerprint).expect("NOCTERM_HUB_FINGERPRINT"),
        username,
        password,
    })
}

/// What the settings file would hold: every document the application saved.
type Saved = Arc<Mutex<Vec<SettingsDocument>>>;

/// The application with no known server, over the real hub adapter.
fn boot(links: &Arc<HubLinks>, cx: &mut TestAppContext) -> (Booted, Saved) {
    let saved = Saved::default();
    let log = saved.clone();
    let writer: SnapshotWriter<SettingsDocument> = Arc::new(move |document| {
        log.lock().unwrap().push(document);
        Box::pin(async { Ok(()) })
    });
    let deps = nocterm_hubs::Deps {
        trust: links.clone(),
        sign_in: links.clone(),
        devices: links.clone(),
        account_keys: Arc::new(nocterm_hub_io::AccountKeys::new(links.clone())),
    };
    let booted = cx.update(|cx| {
        boot_with_ports_and_probe(
            tempfile::tempdir().unwrap(),
            SettingsConfig::with_writer(SettingsDocument::default(), writer)
                .with(HubsSettings::default()),
            (
                Arc::new(MockTransport::default()),
                nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
            ),
            false,
            None,
            None,
            unsupported_sandbox(),
            UserPluginMode::default(),
            Some(deps),
            cx,
        )
    });
    cx.run_until_parked();
    (booted, saved)
}

/// The application composed over the real hub as the application composes it: its accounts,
/// agents, chats and the hub agent connector.
fn boot_live(links: &Arc<HubLinks>, cx: &mut TestAppContext) -> (Booted, Saved) {
    let saved = Saved::default();
    let log = saved.clone();
    let writer: SnapshotWriter<SettingsDocument> = Arc::new(move |document| {
        log.lock().unwrap().push(document);
        Box::pin(async { Ok(()) })
    });
    let booted = cx.update(|cx| {
        boot_with_ports_and_probe(
            tempfile::tempdir().unwrap(),
            SettingsConfig::with_writer(SettingsDocument::default(), writer)
                .with(HubsSettings::default()),
            (
                Arc::new(MockTransport::default()),
                nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
            ),
            false,
            None,
            None,
            unsupported_sandbox(),
            UserPluginMode::default(),
            HubPorts::live(links.clone()),
            cx,
        )
    });
    cx.run_until_parked();
    (booted, saved)
}

/// Opens a window the way the application composes one and returns it.
fn open(booted: &Booted, cx: &mut TestAppContext) -> AnyWindowHandle {
    let (handle, _) = cx.update(|cx| open_workspace(booted, cx));
    cx.run_until_parked();
    show(handle, cx);
    handle
}

fn show(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn workspace_of(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Entity<Workspace> {
    cx.update_window(handle, |_, window, cx| {
        gpui_kit::base::Root::read(window, cx)
            .view()
            .clone()
            .downcast::<Workspace>()
            .unwrap()
    })
    .unwrap()
}

fn dispatch(handle: AnyWindowHandle, action: impl gpui_kit::Action, cx: &mut TestAppContext) {
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

fn click(handle: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
    show(handle, cx);
}

/// Waits for the entrance animation of a dialog, so its buttons can be clicked.
fn settle_dialog(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    show(handle, cx);
    std::thread::sleep(
        *gpui_kit::component::dialog::ANIMATION_DURATION + std::time::Duration::from_millis(50),
    );
    show(handle, cx);
}

/// Runs the windows until `ready` holds, sleeping between looks; fails after [`PATIENCE`].
fn eventually(
    handle: AnyWindowHandle,
    what: &str,
    cx: &mut TestAppContext,
    mut ready: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        show(handle, cx);
        if ready(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn shows(
    handle: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) -> bool {
    let id = id.into();
    cx.update_window(handle, |_, window, _| window.try_find(id).is_some())
        .unwrap()
}

fn press(handle: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    show(handle, cx);
}

fn type_text(handle: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.input(text, cx))
        .unwrap();
    show(handle, cx);
}

/// Signs in through the dialog the sign-in command opens, up to its last step.
fn sign_in_through_the_dialog(
    handle: AnyWindowHandle,
    hub: &Hub,
    saved: &Saved,
    cx: &mut TestAppContext,
) {
    dispatch(handle, nocterm_hubs_ui::SignInToServer, cx);
    settle_dialog(handle, cx);

    type_text(handle, &hub.address.to_string(), cx);
    press(handle, "tab", cx);
    type_text(handle, &hub.fingerprint.to_string(), cx);
    click(handle, "sign-in-continue", cx);
    eventually(handle, "the password step", cx, |cx| {
        shows(handle, "sign-in-password", cx)
    });
    assert!(
        !shows(handle, "sign-in-trust", cx),
        "a fingerprint that matches asks for no decision"
    );

    type_text(handle, &hub.username, cx);
    press(handle, "tab", cx);
    type_text(handle, &hub.password, cx);
    press(handle, "enter", cx);
    eventually(handle, "the last step of the sign-in", cx, |cx| {
        // The first sign-in of an account creates its keys and shows the recovery key once.
        if shows(handle, "sign-in-key-saved", cx) {
            click(handle, "sign-in-key-saved", cx);
        }
        shows(handle, "sign-in-done", cx)
    });

    let servers = saved
        .lock()
        .unwrap()
        .last()
        .map(|document| document.get::<HubsSettings>().servers.clone())
        .unwrap_or_default();
    assert_eq!(servers.len(), 1, "the server is saved in [hubs]");
    assert_eq!(servers[0].id(), hub.id());
    assert_eq!(servers[0].trust, TrustPolicy::Pinned(hub.fingerprint));
}

#[gpui_kit::test]
fn a_user_signs_in_and_signs_out(cx: &mut TestAppContext) {
    let Some(hub) = hub() else { return };
    cx.executor().allow_parking();
    let credentials = tempfile::tempdir().unwrap();
    let links = HubLinks::start(HubLinksConfig {
        credentials_dir: credentials.path().join("hubs"),
        device_name: "gui test".into(),
    })
    .unwrap();
    block_on(links.probe(&hub.address)).expect("the hub answers");

    let (booted, saved) = boot(&links, cx);
    let window = open(&booted, cx);
    sign_in_through_the_dialog(window, &hub, &saved, cx);
    click(window, "sign-in-done", cx);

    sign_out_and_see_the_credentials_go(&links, &hub, credentials.path());
}

fn credential_files(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let entries = std::fs::read_dir(directory).into_iter().flatten().flatten();
    entries
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect()
}

/// Whether the hub counts this device as signed in.
fn signed_in(links: &HubLinks, hub: &Hub) -> bool {
    let endpoint = HubEndpointSpec {
        address: hub.address.clone(),
        trust: TrustPolicy::Pinned(hub.fingerprint),
    };
    block_on(links.connect(endpoint))
        .expect("the hub answers")
        .signed_in
}

/// Signing out ends the sign-in on this device: nothing is left to read, and the hub refuses.
fn sign_out_and_see_the_credentials_go(links: &HubLinks, hub: &Hub, credentials: &std::path::Path) {
    let directory = credentials.join("hubs");
    assert_eq!(credential_files(&directory).len(), 1, "the sign-in is kept");
    assert!(signed_in(links, hub), "the device is signed in");

    block_on(links.sign_out(&hub.id())).unwrap();
    assert!(
        credential_files(&directory).is_empty(),
        "the credentials are gone"
    );
    assert!(!signed_in(links, hub), "the device is signed out");
}
