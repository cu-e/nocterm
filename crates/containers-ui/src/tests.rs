use std::sync::Arc;

use futures::FutureExt as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    TestAppContext, Window, WindowOptions, div, prelude::*,
};
use nocterm_containers::{Action, ContainersError, Engine};
use nocterm_containers_service::{
    ContainerHosts,
    test_support::{EXITED, FakeDockerHost, RUNNING, docker_event},
};
use nocterm_session::{HostExec, Target};
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;
use nocterm_ui::DesignTokens;
use nocterm_workspace::{HostKey, Item, ItemEvent, Panel as _, SessionContext, Workspace};

use crate::{
    ContainersPanel,
    model::{ContainersModel, Status},
};

/// A host with a scripted Docker; see [`FakeDockerHost`].
pub(crate) type FakeHost = FakeDockerHost;

fn announce_exit(host: &FakeHost) {
    host.announce(&docker_event("container", "die", "a1"));
}

struct SessionItem {
    session: SessionContext,
    focus: FocusHandle,
}

impl EventEmitter<ItemEvent> for SessionItem {}

impl Focusable for SessionItem {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SessionItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Item for SessionItem {
    fn tab_title(&self, _: &App) -> SharedString {
        "Session".into()
    }

    fn session(&self, _: &App) -> Option<SessionContext> {
        Some(self.session.clone())
    }
}

pub(crate) struct Fixture {
    pub(crate) window: gpui_kit::AnyWindowHandle,
    pub(crate) workspace: Entity<Workspace>,
    model: Entity<ContainersModel>,
    pub(crate) panel: Entity<ContainersPanel>,
}

pub(crate) fn fixture(cx: &mut TestAppContext, local: Option<Arc<dyn HostExec>>) -> Fixture {
    let fixture = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            DesignTokens::builtin(),
            (SettingsConfig::in_memory(SettingsDocument::default())).install(cx),
            cx,
        );
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps =
                        nocterm_workspace::test_support::deps_for(&[&crate::plugin::PLUGIN], cx);
                    Workspace::new(deps, window, cx)
                })
            })
            .unwrap();
        let model =
            cx.new(|cx| ContainersModel::new(&workspace, None, local, ContainerHosts::new(), cx));
        let panel = cx.new(|cx| ContainersPanel::new(model.clone(), workspace.downgrade(), cx));
        Fixture {
            window,
            workspace,
            model,
            panel,
        }
    });
    cx.run_until_parked();
    fixture
}

fn assert_status(
    cx: &mut TestAppContext,
    fixture: &Fixture,
    expected: impl FnOnce(&Status) -> bool,
) {
    fixture.model.read_with(cx, |model, cx| {
        assert!(
            expected(&model.status(cx)),
            "unexpected status: {:?}",
            model.status(cx)
        );
    });
}

fn badge(cx: &mut TestAppContext, fixture: &Fixture) -> Option<SharedString> {
    cx.update(|cx| fixture.panel.read(cx).badge(cx))
}

pub(crate) fn open_session(cx: &mut TestAppContext, fixture: &Fixture, session: SessionContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        let item = cx.new(|cx| SessionItem {
            session,
            focus: cx.focus_handle(),
        });
        fixture.workspace.update(cx, |workspace, cx| {
            workspace.add_item(item, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
}

pub(crate) fn remote(host: &Arc<FakeHost>, connected: bool) -> SessionContext {
    SessionContext::new(Target::parse("user@host", None).unwrap(), None, connected)
        .with_exec(Some(host.clone() as Arc<dyn HostExec>))
}

#[gpui_kit::test]
fn containers_are_listed_again_when_the_engine_reports_a_change(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    let fixture = fixture(cx, Some(local.clone()));
    assert_status(cx, &fixture, |status| matches!(status, Status::Ready));
    assert_eq!(badge(cx, &fixture).as_deref(), Some("1"));
    assert!(
        local
            .ran()
            .iter()
            .any(|line| line.starts_with("docker events"))
    );

    local.set_containers(EXITED);
    announce_exit(&local);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(badge(cx, &fixture), None, "no running containers, no count");
    assert!(local.following(0), "the same stream is still followed");
}

#[gpui_kit::test]
fn actions_and_programs_run_on_the_followed_host(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    let fixture = fixture(cx, Some(local.clone()));

    let done = fixture.model.update(cx, |model, cx| {
        model.perform(Action::Stop, vec!["a1".into()], cx)
    });
    cx.run_until_parked();
    assert!(matches!(done.now_or_never(), Some(Ok(()))));
    assert!(local.ran().contains(&"docker stop a1".to_owned()));
    // With the events the action caused, once they settle.
    cx.executor()
        .advance_clock(nocterm_containers_service::SETTLE);
    cx.run_until_parked();
    assert_eq!(
        local.count("docker ps"),
        2,
        "the containers are listed again"
    );
    assert_eq!(local.count("docker images"), 1, "the images are not");
    assert!(local.following(0), "over the same event stream");

    let spec = fixture
        .model
        .read_with(cx, |model, cx| {
            model.program("Logs: web".into(), |engine| engine.logs("a1"), cx)
        })
        .unwrap();
    assert_eq!(spec.host.key, HostKey::Local);
    assert!(Arc::ptr_eq(
        &spec.host.exec,
        &(local.clone() as Arc<dyn HostExec>)
    ));
    assert_eq!(spec.program, Engine::Docker.logs("a1"));
}

#[gpui_kit::test]
fn the_panel_follows_the_active_session_without_connections_of_its_own(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    let fixture = fixture(cx, Some(local.clone()));

    let bare = FakeHost::new(false);
    open_session(cx, &fixture, remote(&bare, true));
    assert_status(cx, &fixture, |status| {
        matches!(status, Status::Failed(ContainersError::NotInstalled))
    });
    assert!(!local.following(0), "the previous host is let go");
    assert_eq!(badge(cx, &fixture), None, "another host's count is dropped");
    let asked = bare.ran().len();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        bare.ran().len(),
        asked,
        "a host without an engine is not asked again"
    );

    let offline = FakeHost::new(true);
    open_session(cx, &fixture, remote(&offline, false));
    assert_status(cx, &fixture, |status| matches!(status, Status::Offline));
    assert!(offline.ran().is_empty());
    assert!(
        fixture
            .model
            .read_with(cx, |model, cx| model.program(
                "Logs".into(),
                |e| e.logs("a1"),
                cx
            ))
            .is_none(),
        "no engine is known for it"
    );
}

#[gpui_kit::test]
fn nothing_is_listed_without_a_host(cx: &mut TestAppContext) {
    let fixture = fixture(cx, None);
    assert_status(cx, &fixture, |status| matches!(status, Status::Off));
    let done = fixture.model.update(cx, |model, cx| {
        model.perform(Action::Start, vec!["a1".into()], cx)
    });
    assert!(matches!(
        done.now_or_never(),
        Some(Err(ContainersError::Disconnected))
    ));
}

#[gpui_kit::test]
fn destructive_intents_cannot_move_to_another_host_or_reconnect(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    let fixture = fixture(cx, Some(local.clone()));
    let intent = fixture.model.read_with(cx, |model, cx| {
        model
            .intent(Action::RemoveImage, vec!["same-image".into()], cx)
            .unwrap()
    });
    let another = FakeHost::new(true);
    open_session(cx, &fixture, remote(&another, true));
    let done = fixture
        .model
        .update(cx, |model, cx| model.perform_intent(intent, cx));
    cx.run_until_parked();
    assert!(matches!(
        done.now_or_never(),
        Some(Err(ContainersError::Disconnected))
    ));
    assert!(!local.ran().iter().any(|line| line.contains("same-image")));
    assert!(!another.ran().iter().any(|line| line.contains("same-image")));

    let intent = fixture.model.read_with(cx, |model, cx| {
        model
            .intent(Action::RemoveImage, vec!["same-image".into()], cx)
            .unwrap()
    });
    // Same target, a new connection/executor.
    let reconnected = FakeHost::new(true);
    open_session(cx, &fixture, remote(&reconnected, true));
    let done = fixture
        .model
        .update(cx, |model, cx| model.perform_intent(intent, cx));
    cx.run_until_parked();
    assert!(matches!(
        done.now_or_never(),
        Some(Err(ContainersError::Disconnected))
    ));
    assert!(
        !reconnected
            .ran()
            .iter()
            .any(|line| line.contains("same-image"))
    );
}

#[gpui_kit::test]
fn disconnecting_invalidates_captured_intent_and_rejects_new_actions(cx: &mut TestAppContext) {
    let host = FakeHost::new(true);
    let fixture = fixture(cx, None);
    open_session(cx, &fixture, remote(&host, true));
    let intent = fixture.model.read_with(cx, |model, cx| {
        model.intent(Action::Remove, vec!["a1".into()], cx).unwrap()
    });
    open_session(cx, &fixture, remote(&host, false));
    assert!(
        fixture
            .model
            .read_with(cx, |model, cx| model.intent(
                Action::Remove,
                vec!["a1".into()],
                cx
            ))
            .is_none()
    );
    let done = fixture
        .model
        .update(cx, |model, cx| model.perform_intent(intent, cx));
    cx.run_until_parked();
    assert!(matches!(
        done.now_or_never(),
        Some(Err(ContainersError::Disconnected))
    ));
    open_session(cx, &fixture, remote(&host, true));
    assert!(!host.ran().iter().any(|line| line == "docker rm -f a1"));
}

#[gpui_kit::test]
fn malformed_successful_listing_remains_a_retryable_failure(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    local.set_containers("not JSON");
    let fixture = fixture(cx, Some(local.clone()));
    assert_status(cx, &fixture, |status| {
        let Status::Failed(ContainersError::Parse(error)) = status else {
            return false;
        };
        matches!(
            error,
            nocterm_containers::ListingError::Json { line: 1, .. }
        )
    });
    let attempts = local.ran().len();
    local.set_containers(RUNNING);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(10));
    cx.run_until_parked();
    assert!(local.ran().len() > attempts);
    assert_status(cx, &fixture, |status| matches!(status, Status::Ready));
    assert_eq!(badge(cx, &fixture).as_deref(), Some("1"));
}

#[gpui_kit::test]
fn windows_on_the_same_host_share_one_watch(cx: &mut TestAppContext) {
    let first = FakeHost::new(true);
    let second = FakeHost::new(true);
    let shared = crate::install(Some(first.clone()));
    let other = crate::install(Some(second.clone()));
    for installed in [&shared, &other, &shared] {
        let fixture = fixture(cx, None);
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.workspace.update(cx, |workspace, cx| {
                crate::register(installed, workspace, window, cx).unwrap();
            });
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(first.count("docker events"), 1, "two windows, one watch");
    assert_eq!(first.count("docker ps"), 1);
    assert_eq!(second.count("docker events"), 1);
    assert!(first.following(0));
    assert!(second.following(0));
}

#[gpui_kit::test]
fn health_checks_do_not_list_the_host_again(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    let fixture = fixture(cx, Some(local.clone()));
    for _ in 0..10 {
        for action in [
            "exec_create: pg_isready",
            "exec_start: pg_isready",
            "exec_die",
        ] {
            local.announce(&docker_event("container", action, "pg"));
        }
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3));
        cx.run_until_parked();
    }
    assert_eq!(local.count("docker ps"), 1);
    assert_eq!(local.count("docker images"), 1);
    assert_eq!(badge(cx, &fixture).as_deref(), Some("1"));
}
