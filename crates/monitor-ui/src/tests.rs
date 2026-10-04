use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::executor::block_on;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    TestAppContext, Window, WindowOptions, div, prelude::*,
};
use nocterm_monitor::MonitorMetric;
use nocterm_session::{ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec, Target};
use nocterm_settings::{MonitorSettings, Settings};
use nocterm_ui::{DesignTokens, SettingsStore, edit_settings};
use nocterm_workspace::{Item, ItemEvent, SessionContext, Workspace};

use crate::model::{Host, HostKey, HostMonitor, Plan, Status};

/// A host that answers `uname` and keeps every other program running until
/// the test writes to it.
struct FakeHost {
    system: &'static str,
    requests: Mutex<Vec<ExecRequest>>,
    running: Mutex<Vec<ExecSink>>,
}

impl FakeHost {
    fn new(system: &'static str) -> Arc<Self> {
        Arc::new(Self {
            system,
            requests: Mutex::default(),
            running: Mutex::default(),
        })
    }

    fn programs(&self) -> Vec<String> {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .map(|request| request.program.clone())
            .collect()
    }

    /// The script of the latest watch.
    fn script(&self) -> String {
        let requests = self.requests.lock().unwrap();
        let request = requests.last().expect("a watch was started");
        String::from_utf8_lossy(request.stdin.as_deref().unwrap_or_default()).into_owned()
    }

    /// Prints `text` from the latest watch.
    fn print(&self, text: &str) {
        let running = self.running.lock().unwrap();
        assert!(block_on(
            running.last().unwrap().send(text.as_bytes().to_vec())
        ));
    }

    /// Whether the watch started `index`th is still being read.
    fn reading(&self, index: usize) -> bool {
        !self.running.lock().unwrap()[index].is_closed()
    }
}

impl HostExec for FakeHost {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let uname = request.program == "uname";
        self.requests.lock().unwrap().push(request);
        let (sink, output) = ExecOutput::channel();
        if !uname {
            self.running.lock().unwrap().push(sink);
            return Box::pin(async { Ok(output) });
        }
        let system = format!("{}\n", self.system);
        Box::pin(async move {
            sink.send(system.into_bytes()).await;
            Ok(output)
        })
    }
}

fn target(text: &str) -> Target {
    Target::parse(text, None).unwrap()
}

fn remote(host: &Arc<FakeHost>, connected: bool) -> SessionContext {
    SessionContext::new(target("user@host"), None, connected)
        .with_exec(Some(host.clone() as Arc<dyn HostExec>))
}

#[test]
fn closed_details_collect_only_the_status_bar_slowly() {
    let settings = MonitorSettings::default();
    let closed = Plan::new(&settings, false).unwrap();
    assert_eq!(
        closed.metrics.iter().collect::<Vec<_>>(),
        [MonitorMetric::Cpu, MonitorMetric::Memory]
    );
    assert_eq!(closed.interval, Duration::from_secs(30));

    let open = Plan::new(&settings, true).unwrap();
    assert_eq!(open.metrics.iter().count(), MonitorMetric::ALL.len());
    assert_eq!(open.interval, Duration::from_secs(2));

    let hidden = MonitorSettings {
        status_bar: Vec::new(),
        ..MonitorSettings::default()
    };
    assert_eq!(
        Plan::new(&hidden, false),
        None,
        "nothing shown, nothing read"
    );
    assert!(Plan::new(&hidden, true).is_some());
}

#[test]
fn sessions_that_run_programs_are_watched_remotely() {
    let settings = MonitorSettings::default();
    let host = FakeHost::new("Linux");
    let local: Arc<dyn HostExec> = FakeHost::new("Linux");

    let watched = Host::resolve(Some(&remote(&host, true)), Some(&local), &settings).unwrap();
    assert_eq!(watched.key, HostKey::Remote(target("user@host")));

    let without_exec = SessionContext::new(target("user@other"), None, true);
    let watched = Host::resolve(Some(&without_exec), Some(&local), &settings).unwrap();
    assert_eq!(watched.key, HostKey::Local);
    assert_eq!(
        Host::resolve(None, Some(&local), &settings).unwrap().key,
        HostKey::Local
    );
    assert!(Host::resolve(None, None, &settings).is_none());

    let remote_only = MonitorSettings {
        local: false,
        ..MonitorSettings::default()
    };
    assert!(Host::resolve(None, Some(&local), &remote_only).is_none());
    let off = MonitorSettings {
        enabled: false,
        ..MonitorSettings::default()
    };
    assert!(Host::resolve(Some(&remote(&host, true)), Some(&local), &off).is_none());
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

/// A window with an empty workspace and a monitor of it.
fn fixture(
    cx: &mut TestAppContext,
    local: Option<Arc<dyn HostExec>>,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<Workspace>,
    Entity<HostMonitor>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            DesignTokens::builtin(),
            SettingsStore::in_memory(Settings::default()),
            cx,
        );
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(window, cx))
            })
            .unwrap();
        let monitor = cx.new(|cx| HostMonitor::new(&workspace, None, local, cx));
        (window, workspace, monitor)
    })
}

fn open_session(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    workspace: &Entity<Workspace>,
    session: SessionContext,
) -> Entity<SessionItem> {
    let item = cx
        .update_window(window, |_, window, cx| {
            let item = cx.new(|cx| SessionItem {
                session,
                focus: cx.focus_handle(),
            });
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(item.clone(), window, cx);
            });
            item
        })
        .unwrap();
    cx.run_until_parked();
    item
}

fn status(cx: &mut TestAppContext, monitor: &Entity<HostMonitor>) -> Status {
    monitor.read_with(cx, |monitor, _| monitor.status().clone())
}

#[gpui_kit::test]
fn the_monitor_follows_the_active_session_and_collects_what_is_shown(cx: &mut TestAppContext) {
    let (window, workspace, monitor) = fixture(cx, None);
    assert_eq!(
        status(cx, &monitor),
        Status::Off,
        "no session, no local runner"
    );

    let host = FakeHost::new("Linux");
    let item = open_session(cx, window, &workspace, remote(&host, true));
    assert_eq!(host.programs(), ["uname", "sh"]);
    assert_eq!(status(cx, &monitor), Status::Starting);
    let script = host.script();
    assert!(script.contains("@@stat") && script.contains("@@meminfo"));
    assert!(
        !script.contains("@@netdev"),
        "closed details read the status bar only"
    );

    host.print("@@meminfo\nMemTotal: 1000\nMemAvailable: 250\n@@end\n");
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Running);
    let memory = monitor.read_with(cx, |monitor, _| {
        monitor.sampler().unwrap().snapshot().memory
    });
    assert_eq!(memory.unwrap().percent(), 75.0);

    monitor.update(cx, |monitor, cx| monitor.set_open(true, cx));
    cx.run_until_parked();
    assert_eq!(
        host.programs(),
        ["uname", "sh", "sh"],
        "the platform is remembered"
    );
    assert!(
        host.script().contains("@@netdev"),
        "open details read everything"
    );
    assert!(!host.reading(0), "the slow watch stopped");
    assert!(host.reading(1));

    item.update(cx, |item, cx| {
        item.session.connected = false;
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Offline);
    assert!(!host.reading(1), "a disconnected host is not read");
}

#[gpui_kit::test]
fn settings_switch_the_monitor_off_and_hosts_without_a_script_say_so(cx: &mut TestAppContext) {
    let local = FakeHost::new("Linux");
    let (window, workspace, monitor) = fixture(cx, Some(local.clone()));
    cx.run_until_parked();
    assert_eq!(
        monitor.read_with(cx, |monitor, _| monitor.host().cloned()),
        Some(HostKey::Local)
    );
    assert_eq!(local.programs(), ["sh"], "this computer needs no detection");

    let mac = FakeHost::new("Darwin");
    open_session(cx, window, &workspace, remote(&mac, true));
    assert_eq!(status(cx, &monitor), Status::Unsupported("Darwin".into()));
    assert!(!local.reading(0));

    cx.update(|cx| edit_settings(cx, |settings| settings.monitor.enabled = false).detach());
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Off);
    assert_eq!(mac.programs(), ["uname"]);
}
