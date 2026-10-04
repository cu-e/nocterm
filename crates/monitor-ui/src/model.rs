//! Which host the monitor watches, and the one script that watches it.
//!
//! The model follows the workspace's active session. A remote session that
//! can run programs is watched over its own connection; anything else is
//! this computer, when the settings allow. Only one script runs at a time,
//! and it collects only what is on screen: the status bar's metrics while
//! the details are closed, at the slow interval, and everything the details
//! show while they are open, at the fast one.

use std::{sync::Arc, time::Duration};

use gpui_kit::{App, AsyncApp, Context, Entity, EventEmitter, Subscription, Task, WeakEntity};
use nocterm_monitor::{MetricSet, Platform, Sampler, Watch, detect, local_platform};
use nocterm_session::{ExecError, HostExec, Target};
use nocterm_settings::MonitorSettings;
use nocterm_ui::{ActiveSettings as _, SettingsStore};
use nocterm_workspace::{SessionContext, Workspace, WorkspaceEvent};

/// How long a failed or ended watch waits before it is tried again.
const RETRY: Duration = Duration::from_secs(10);
/// Hosts whose platform and history are remembered for switching back.
const REMEMBERED_HOSTS: usize = 8;

/// Which machine a reading is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostKey {
    /// The computer nocterm runs on.
    Local,
    Remote(Target),
}

impl HostKey {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Local => "This computer".into(),
            Self::Remote(target) => target.to_string(),
        }
    }
}

/// The machine to watch and how to reach it.
#[derive(Clone)]
pub(crate) struct Host {
    pub(crate) key: HostKey,
    exec: Arc<dyn HostExec>,
    connected: bool,
}

impl Host {
    /// The host behind `session`: its own connection when it can run
    /// programs, otherwise this computer, if `settings` allow watching it.
    pub(crate) fn resolve(
        session: Option<&SessionContext>,
        local: Option<&Arc<dyn HostExec>>,
        settings: &MonitorSettings,
    ) -> Option<Self> {
        if !settings.enabled {
            return None;
        }
        if let Some((session, exec)) =
            session.and_then(|session| Some((session, session.exec.clone()?)))
        {
            return Some(Self {
                key: HostKey::Remote(session.target.clone()),
                exec,
                connected: session.connected,
            });
        }
        settings.local.then_some(())?;
        Some(Self {
            key: HostKey::Local,
            exec: local?.clone(),
            connected: true,
        })
    }

    fn same_as(&self, other: &Self) -> bool {
        self.key == other.key
            && self.connected == other.connected
            && Arc::ptr_eq(&self.exec, &other.exec)
    }
}

/// What to collect and how often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) metrics: MetricSet,
    pub(crate) interval: Duration,
}

impl Plan {
    /// The status bar alone while the details are closed; both, faster,
    /// while they are open. `None` when nothing is shown.
    pub(crate) fn new(settings: &MonitorSettings, open: bool) -> Option<Self> {
        let mut metrics: MetricSet = settings.status_bar.iter().collect();
        let mut seconds = settings.interval_secs;
        if open {
            metrics = metrics.union(settings.details.iter().collect());
            seconds = settings.detail_interval_secs;
        }
        (!metrics.is_empty()).then(|| Self {
            metrics,
            interval: Duration::from_secs(u64::from(seconds.max(1))),
        })
    }
}

/// Where the watch of the current host stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// Monitoring is off, or nothing is shown.
    Off,
    /// The session is not connected.
    Offline,
    /// Asking the host what it is.
    Detecting,
    /// Waiting for the first readings.
    Starting,
    Running,
    /// The host is of a kind there is no script for.
    Unsupported(String),
    /// The last attempt failed; another follows shortly.
    Failed(String),
}

/// A remembered host: what it is and what it looked like.
struct Record {
    key: HostKey,
    platform: Option<Platform>,
    sampler: Sampler,
}

pub(crate) struct HostMonitor {
    local: Option<Arc<dyn HostExec>>,
    session: Option<SessionContext>,
    host: Option<Host>,
    open: bool,
    status: Status,
    /// Most recently used first; the current host, when there is one, leads.
    records: Vec<Record>,
    /// The host and plan the task was started for.
    wanted: Option<(Host, Plan)>,
    task: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

/// Fired whenever new readings or a new status are available.
pub(crate) struct Changed;

impl EventEmitter<Changed> for HostMonitor {}

impl HostMonitor {
    pub(crate) fn new(
        workspace: &Entity<Workspace>,
        session: Option<SessionContext>,
        local: Option<Arc<dyn HostExec>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let follow = cx.subscribe(workspace, |_, workspace, event, cx| {
            if *event == WorkspaceEvent::ActiveSessionChanged {
                let workspace = workspace.downgrade();
                let monitor = cx.entity().downgrade();
                // The workspace may still be mid-update when it announces.
                cx.defer(move |cx| {
                    let Some(workspace) = workspace.upgrade() else {
                        return;
                    };
                    let session = workspace.read(cx).active_session(cx);
                    let _ = monitor.update(cx, |this, cx| this.follow(session, cx));
                });
            }
        });
        let settings = cx.observe_global::<SettingsStore>(|this, cx| this.refresh(cx));
        let mut this = Self {
            local,
            session,
            host: None,
            open: false,
            status: Status::Off,
            records: Vec::new(),
            wanted: None,
            task: None,
            _subscriptions: [follow, settings],
        };
        this.refresh(cx);
        this
    }

    pub(crate) fn status(&self) -> &Status {
        &self.status
    }

    pub(crate) fn host(&self) -> Option<&HostKey> {
        self.host.as_ref().map(|host| &host.key)
    }

    /// What the current host looked like at the last reading.
    pub(crate) fn sampler(&self) -> Option<&Sampler> {
        let key = &self.host.as_ref()?.key;
        self.records
            .first()
            .filter(|record| record.key == *key)
            .map(|record| &record.sampler)
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// The details opened or closed: collect more, or less.
    pub(crate) fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open != open {
            self.open = open;
            self.refresh(cx);
        }
    }

    fn follow(&mut self, session: Option<SessionContext>, cx: &mut Context<Self>) {
        self.session = session;
        self.refresh(cx);
    }

    /// Starts, restarts or stops the watch to match the session, the
    /// settings and the details.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let settings = cx.settings().monitor.clone();
        let host = Host::resolve(self.session.as_ref(), self.local.as_ref(), &settings);
        let plan = Plan::new(&settings, self.open);
        let history = settings.history_points as usize;
        for record in &mut self.records {
            record.sampler.set_history_points(history);
        }

        let wanted = host.clone().zip(plan);
        let unchanged = match (&self.wanted, &wanted) {
            (Some((was, before)), Some((host, plan))) => was.same_as(host) && before == plan,
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }
        self.task = None;
        self.wanted = wanted;
        self.host = host;
        let Some((host, plan)) = self.wanted.clone() else {
            self.set_status(Status::Off, cx);
            return;
        };
        self.remember(&host.key, history);
        if !host.connected {
            self.set_status(Status::Offline, cx);
            return;
        }
        let platform = match host.key {
            HostKey::Local => Some(local_platform()),
            HostKey::Remote(_) => self.records[0].platform.clone(),
        };
        self.set_status(
            if platform.is_some() {
                Status::Starting
            } else {
                Status::Detecting
            },
            cx,
        );
        self.task = Some(cx.spawn(async move |this, cx| {
            watch(this, host, platform, plan, cx).await;
        }));
    }

    /// Moves the record of `key` to the front, creating it if need be.
    fn remember(&mut self, key: &HostKey, history: usize) {
        let record = match self.records.iter().position(|record| record.key == *key) {
            Some(index) => self.records.remove(index),
            None => Record {
                key: key.clone(),
                platform: None,
                sampler: Sampler::new(history),
            },
        };
        self.records.insert(0, record);
        self.records.truncate(REMEMBERED_HOSTS);
    }

    /// The record of the watched host, if it is still `key`.
    fn current(&mut self, key: &HostKey) -> Option<&mut Record> {
        self.records.first_mut().filter(|record| record.key == *key)
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.emit(Changed);
            cx.notify();
        }
    }
}

/// Detects the host if need be, then runs its script, again after failures.
async fn watch(
    this: WeakEntity<HostMonitor>,
    host: Host,
    mut platform: Option<Platform>,
    plan: Plan,
    cx: &mut AsyncApp,
) {
    let key = host.key.clone();
    loop {
        let outcome = run(&this, &host, &mut platform, plan, cx).await;
        let retry = this.update(cx, |this, cx| {
            let status = match outcome {
                Err(ExecError::Unsupported) => {
                    let name = match &platform {
                        Some(Platform::Unsupported(name)) => name.clone(),
                        _ => "unknown".into(),
                    };
                    this.set_status(Status::Unsupported(name), cx);
                    return false;
                }
                Err(ExecError::Disconnected) => "The connection closed.".into(),
                Err(ExecError::Failed(error)) => error,
                Ok(()) => "The monitor script stopped.".into(),
            };
            if this.current(&key).is_some() {
                this.set_status(Status::Failed(status), cx);
            }
            true
        });
        if !matches!(retry, Ok(true)) {
            return;
        }
        cx.background_executor().timer(RETRY).await;
    }
}

/// One attempt: detection, then readings until the script ends.
async fn run(
    this: &WeakEntity<HostMonitor>,
    host: &Host,
    platform: &mut Option<Platform>,
    plan: Plan,
    cx: &mut AsyncApp,
) -> Result<(), ExecError> {
    let detected = match platform.clone() {
        Some(platform) => platform,
        None => {
            let detected = detect(host.exec.as_ref()).await?;
            *platform = Some(detected.clone());
            let key = host.key.clone();
            let remembered = detected.clone();
            this.update(cx, |this, cx| {
                if let Some(record) = this.current(&key) {
                    record.platform = Some(remembered);
                    this.set_status(Status::Starting, cx);
                }
            })
            .map_err(|_| ExecError::Disconnected)?;
            detected
        }
    };
    let mut watch = Watch::start(host.exec.as_ref(), detected, plan.metrics, plan.interval).await?;
    while let Some(reading) = watch.next().await {
        let key = host.key.clone();
        let applied = this.update(cx, |this, cx| {
            let Some(record) = this.current(&key) else {
                return false;
            };
            record.sampler.apply(reading);
            this.status = Status::Running;
            cx.emit(Changed);
            cx.notify();
            true
        });
        if !matches!(applied, Ok(true)) {
            return Ok(());
        }
    }
    Ok(())
}

/// The program runner for this computer, installed by the application.
pub(crate) struct LocalExec(pub(crate) Arc<dyn HostExec>);

impl gpui_kit::Global for LocalExec {}

pub(crate) fn local_exec(cx: &App) -> Option<Arc<dyn HostExec>> {
    cx.try_global::<LocalExec>().map(|local| local.0.clone())
}
