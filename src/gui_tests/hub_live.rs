//! A user signs in to a real hub, works in a session of it and signs out, through the windows.
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
use super::hub_sessions::{click, dispatch, open, settle_dialog, show, workspace_of};
use super::*;
use crate::bootstrap::Booted;
use crate::shell_setup::UserPluginMode;
use futures::executor::block_on;
use nocterm_hub_io::{HubLinks, HubLinksConfig};
use nocterm_hubs_model::{
    Fingerprint, HubAddress, HubId, HubSessionId, HubSessions as _, HubSignIn as _, HubTrust as _,
    HubsSettings, OrgId, TrustPolicy,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SnapshotWriter};
use nocterm_workspace::CloseTab;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// How long the hub, the node and the shell get to answer.
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
        sessions: links.clone(),
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

/// Ends the sessions a failing test leaves on the hub.
struct Tidy {
    links: Arc<HubLinks>,
    hub: HubId,
    org: OrgId,
    before: HashSet<HubSessionId>,
}

impl Tidy {
    fn sessions(&self) -> Vec<nocterm_hubs_model::HubSessionSummary> {
        block_on(self.links.sessions(&self.hub, &self.org)).unwrap_or_default()
    }

    /// The session that was not there when the test began.
    fn mine(&self) -> Option<nocterm_hubs_model::HubSessionSummary> {
        self.sessions()
            .into_iter()
            .find(|session| !self.before.contains(&session.id))
    }
}

impl Drop for Tidy {
    fn drop(&mut self) {
        for session in self.sessions() {
            if !self.before.contains(&session.id) {
                let _ = block_on(self.links.end(&self.hub, &session.id));
            }
        }
    }
}

/// The text the active tab shows, scrollback included.
fn screen(handle: AnyWindowHandle, cx: &mut TestAppContext) -> String {
    let workspace = workspace_of(handle, cx);
    cx.update(|cx| {
        let view = workspace
            .read(cx)
            .active_item(cx)?
            .view()
            .downcast::<TerminalView>()
            .ok()?;
        let terminal = view.read(cx).terminal().clone();
        terminal.update(cx, |terminal, cx| terminal.select_all(cx));
        let text = terminal.read(cx).emulator().selection_text();
        terminal.update(cx, |terminal, cx| terminal.clear_selection(cx));
        text
    })
    .unwrap_or_default()
}

fn tabs(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    let workspace = workspace_of(handle, cx);
    cx.update(|cx| workspace.read(cx).items(cx).count())
}

/// Types a command into the active tab, which shows its own focus.
fn run_in_tab(handle: AnyWindowHandle, command: &str, cx: &mut TestAppContext) {
    let workspace = workspace_of(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        let view = workspace.read(cx).active_item(cx).unwrap().view();
        let view = view.downcast::<TerminalView>().unwrap();
        window.focus(&view.read(cx).focus_handle(cx), cx);
    })
    .unwrap();
    type_text(handle, command, cx);
    press(handle, "enter", cx);
}

#[gpui_kit::test]
fn a_user_signs_in_works_in_a_session_and_signs_out(cx: &mut TestAppContext) {
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

    // The session panel of this window lists the node of the hub.
    click(window, "sign-in-done", cx);
    dispatch(window, nocterm_hubs_ui::OpenHubSessions, cx);
    let account = block_on(links.account(&hub.id())).unwrap();
    let org = account.orgs.first().expect("an organization").id.clone();
    eventually(window, "the sessions panel", cx, |cx| {
        shows(window, "hub-sessions-new", cx)
    });
    let nodes = block_on(links.nodes(&hub.id(), &org)).unwrap();
    let node = nodes.iter().find(|node| node.name == "demo-node").unwrap();
    eventually(window, "the node in the panel", cx, |cx| {
        shows(window, format!("hub-node-{}", node.id), cx)
    });

    let tidy = Tidy {
        before: block_on(links.sessions(&hub.id(), &org))
            .unwrap()
            .into_iter()
            .map(|session| session.id)
            .collect(),
        links: links.clone(),
        hub: hub.id(),
        org,
    };
    let marker = format!(
        "nocterm-live-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let id = start_a_session_and_leave_a_marker(window, &tidy, &marker, cx);
    return_to_the_session_and_end_it(window, &tidy, &id, &marker, cx);

    sign_out_and_see_the_credentials_go(&links, &hub, credentials.path());
    drop(tidy);
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

/// Signing out ends the sign-in on this device: nothing is left to read, and the hub refuses.
fn sign_out_and_see_the_credentials_go(links: &HubLinks, hub: &Hub, credentials: &std::path::Path) {
    let directory = credentials.join("hubs");
    assert_eq!(credential_files(&directory).len(), 1, "the sign-in is kept");

    block_on(links.sign_out(&hub.id())).unwrap();
    assert!(
        credential_files(&directory).is_empty(),
        "the credentials are gone"
    );
    assert!(matches!(
        block_on(links.account(&hub.id())),
        Err(nocterm_hubs_model::HubError::NotSignedIn)
    ));
}

/// "New session", a shell answers a command, and closing the tab leaves the session listed.
fn start_a_session_and_leave_a_marker(
    window: AnyWindowHandle,
    tidy: &Tidy,
    marker: &str,
    cx: &mut TestAppContext,
) -> HubSessionId {
    click(window, "hub-sessions-new", cx);
    eventually(window, "the tab of the new session", cx, |cx| {
        tabs(window, cx) == 1
    });
    // The shell computes the marker, so seeing it means the command ran and was not just echoed.
    let command = format!("echo {marker}$((20+22))");
    eventually(window, "a shell prompt", cx, |cx| {
        !screen(window, cx).trim().is_empty()
    });
    run_in_tab(window, &command, cx);
    let answer = format!("{marker}42");
    eventually(window, "the answer of the shell", cx, |cx| {
        screen(window, cx).contains(&answer)
    });

    let session = tidy.mine().expect("the session exists on the hub");
    dispatch(window, CloseTab, cx);
    assert_eq!(tabs(window, cx), 0, "the tab is closed");
    click(window, "hub-sessions-refresh", cx);
    eventually(window, "the session in the list", cx, |cx| {
        shows(window, format!("hub-session-{}", session.id), cx)
    });
    assert!(
        tidy.sessions().iter().any(|listed| listed.id == session.id),
        "closing the tab only detaches"
    );
    session.id
}

/// Opening the session again restores its screen; ending it removes it from the list.
fn return_to_the_session_and_end_it(
    window: AnyWindowHandle,
    tidy: &Tidy,
    id: &HubSessionId,
    marker: &str,
    cx: &mut TestAppContext,
) {
    click(window, format!("hub-session-open-{id}"), cx);
    let answer = format!("{marker}42");
    eventually(window, "the restored screen", cx, |cx| {
        screen(window, cx).contains(&answer)
    });

    click(window, format!("hub-session-end-{id}"), cx);
    settle_dialog(window, cx);
    click(window, "end-session-confirm", cx);
    eventually(window, "the session to leave the list", cx, |cx| {
        !shows(window, format!("hub-session-{id}"), cx)
    });
    assert!(
        tidy.sessions().iter().all(|listed| &listed.id != id),
        "the hub ended it"
    );
}
