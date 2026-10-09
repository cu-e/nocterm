use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    TestAppContext, Window, WindowOptions, div, prelude::*,
};
use nocterm_containers::{Action, ContainersError, Engine};
use nocterm_session::{ExecExit, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec, Target};
use nocterm_settings::SettingsDocument;
use nocterm_ui::{DesignTokens, SettingsStore};
use nocterm_workspace::{HostKey, Item, ItemEvent, Panel as _, SessionContext, Workspace};

use crate::{
    ContainersPanel,
    model::{ContainersModel, Status},
};

const RUNNING: &str =
    r#"{"ID":"a1","Image":"nginx","Names":"web","State":"running","Status":"Up 2 hours"}"#;
const EXITED: &str =
    r#"{"ID":"a1","Image":"nginx","Names":"web","State":"exited","Status":"Exited (0)"}"#;

/// A host whose `docker` lists `containers`, keeps `docker events` running
/// until the test prints to it, and does every other command it is asked.
pub(crate) struct FakeHost {
    docker: bool,
    containers: Mutex<&'static str>,
    ran: Mutex<Vec<String>>,
    events: Mutex<Vec<ExecSink>>,
}

impl FakeHost {
    pub(crate) fn new(docker: bool) -> Arc<Self> {
        Arc::new(Self {
            docker,
            containers: Mutex::new(RUNNING),
            ran: Mutex::default(),
            events: Mutex::default(),
        })
    }

    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }

    /// Reports a change from the latest event stream.
    fn announce(&self) {
        let events = self.events.lock().unwrap();
        let sink = events.last().expect("events are followed");
        assert_eq!(
            sink.send(b"container die\n".to_vec()).now_or_never(),
            Some(true)
        );
    }

    /// Whether the event stream started `index`th is still being read.
    fn following(&self, index: usize) -> bool {
        !self.events.lock().unwrap()[index].is_closed()
    }
}

impl HostExec for FakeHost {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let line = request.command_line();
        self.ran.lock().unwrap().push(line.clone());
        let (sink, output) = ExecOutput::channel();
        if self.docker && line.starts_with("docker events") {
            self.events.lock().unwrap().push(sink);
            return Box::pin(async { Ok(output) });
        }
        let (stdout, status) = match () {
            _ if !self.docker => ("", 127),
            _ if line.starts_with("docker ps") => (*self.containers.lock().unwrap(), 0),
            _ if line.starts_with("docker images") => ("", 0),
            _ if line.starts_with("docker") => ("", 0),
            _ => ("", 127),
        };
        assert_eq!(
            sink.send(stdout.as_bytes().to_vec()).now_or_never(),
            Some(true)
        );
        sink.finish(ExecExit {
            status: Some(status),
            stderr: String::new(),
        });
        Box::pin(async { Ok(output) })
    }
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
            SettingsStore::in_memory(SettingsDocument::default()),
            cx,
        );
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(window, cx))
            })
            .unwrap();
        let model = cx.new(|cx| ContainersModel::new(&workspace, None, local, cx));
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

fn status(cx: &mut TestAppContext, fixture: &Fixture) -> Status {
    fixture
        .model
        .read_with(cx, |model, _| model.status().clone())
}

fn badge(cx: &mut TestAppContext, fixture: &Fixture) -> Option<SharedString> {
    cx.update(|cx| fixture.panel.read(cx).badge(cx))
}

fn open_session(cx: &mut TestAppContext, fixture: &Fixture, session: SessionContext) {
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

fn remote(host: &Arc<FakeHost>, connected: bool) -> SessionContext {
    SessionContext::new(Target::parse("user@host", None).unwrap(), None, connected)
        .with_exec(Some(host.clone() as Arc<dyn HostExec>))
}

#[gpui_kit::test]
fn containers_are_listed_again_when_the_engine_reports_a_change(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    let fixture = fixture(cx, Some(local.clone()));
    assert_eq!(status(cx, &fixture), Status::Ready);
    assert_eq!(badge(cx, &fixture).as_deref(), Some("1"));
    assert!(
        local
            .ran()
            .iter()
            .any(|line| line.starts_with("docker events"))
    );

    *local.containers.lock().unwrap() = EXITED;
    local.announce();
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
    assert_eq!(done.now_or_never(), Some(Ok(())));
    assert!(local.ran().contains(&"docker stop a1".to_owned()));
    assert!(!local.following(0), "a refresh follows the engine anew");

    let spec = fixture
        .model
        .read_with(cx, |model, _| {
            model.program("Logs: web".into(), |engine| engine.logs("a1"))
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
    assert_eq!(
        status(cx, &fixture),
        Status::Failed(ContainersError::NotInstalled)
    );
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
    assert_eq!(status(cx, &fixture), Status::Offline);
    assert!(offline.ran().is_empty());
    assert!(
        fixture
            .model
            .read_with(cx, |model, _| model
                .program("Logs".into(), |e| e.logs("a1")))
            .is_none(),
        "no engine is known for it"
    );
}

#[gpui_kit::test]
fn nothing_is_listed_without_a_host(cx: &mut TestAppContext) {
    let fixture = fixture(cx, None);
    assert_eq!(status(cx, &fixture), Status::Off);
    let done = fixture.model.update(cx, |model, cx| {
        model.perform(Action::Start, vec!["a1".into()], cx)
    });
    assert_eq!(
        done.now_or_never(),
        Some(Err(ContainersError::Disconnected))
    );
}
