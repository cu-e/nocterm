//! Selected surface resource identity and one generation-bound monitoring watch.
//!
//! General contexts retain explicitly supplied local policy. Project contexts
//! observe only their original inner session; unavailable resources never fall
//! back to an outer or local executor. History is bounded per surface/resource.

use std::{sync::Arc, time::Duration};

use crate::MonitorSettings;
#[cfg(test)]
pub(crate) use crate::source::watched_host;
use crate::source::{MonitorKey, WatchHost, watched_source};
use gpui_kit::{App, AsyncApp, Context, Entity, EventEmitter, Subscription, Task, WeakEntity};
use nocterm_monitor::{MetricSet, Platform, Sampler, Watch, detect, local_platform};
use nocterm_session::{ExecError, HostExec};
use nocterm_workspace::{HostKey, MonitorSource, Workspace, WorkspaceEvent};

/// How long a failed or ended watch waits before it is tried again.
const RETRY: Duration = Duration::from_secs(10);
/// Hosts whose platform and history are remembered for switching back.
const REMEMBERED_HOSTS: usize = 8;

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
    key: MonitorKey,
    platform: Option<Platform>,
    sampler: Sampler,
}

pub(crate) struct HostMonitor {
    settings: nocterm_store::SettingsHandle,
    local: Option<Arc<dyn HostExec>>,
    source: MonitorSource,
    host: Option<WatchHost>,
    open: bool,
    status: Status,
    /// Most recently used first; the current host, when there is one, leads.
    records: Vec<Record>,
    /// The host and plan the task was started for.
    wanted: Option<(WatchHost, Plan)>,
    task: Option<Task<()>>,
    generation: u64,
    _subscriptions: [Subscription; 2],
}

/// Fired whenever new readings or a new status are available.
pub(crate) struct Changed;

impl EventEmitter<Changed> for HostMonitor {}

impl HostMonitor {
    pub(crate) fn new(
        workspace: &Entity<Workspace>,
        source: MonitorSource,
        installed: &crate::Installed,
        cx: &mut Context<Self>,
    ) -> Self {
        let follow = cx.subscribe(workspace, |_, workspace, event, cx| {
            if matches!(
                event,
                WorkspaceEvent::ContextChanged
                    | WorkspaceEvent::ActiveSessionChanged
                    | WorkspaceEvent::ActiveItemChanged
                    | WorkspaceEvent::ItemsChanged
            ) {
                let workspace = workspace.downgrade();
                let this = cx.entity().downgrade();
                cx.defer(move |cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        let source = workspace.read(cx).monitor_source(cx);
                        let _ = this.update(cx, |this, cx| this.follow(source, cx));
                    }
                });
            }
        });
        let weak = cx.weak_entity();
        let observation = installed.settings().observe(cx, move |cx| {
            let _ = weak.update(cx, |this, cx| this.refresh(cx));
        });
        let mut this = Self {
            local: installed.local(),
            settings: installed.settings().clone(),
            source,
            host: None,
            open: false,
            status: Status::Off,
            records: Vec::new(),
            wanted: None,
            task: None,
            generation: 0,
            _subscriptions: [follow, observation],
        };
        this.refresh(cx);
        this
    }

    pub(crate) fn settings<'a>(&self, cx: &'a App) -> &'a MonitorSettings {
        self.settings.setting::<MonitorSettings>(cx)
    }
    pub(crate) fn status(&self) -> &Status {
        &self.status
    }

    #[cfg(test)]
    pub(crate) fn host(&self) -> Option<&HostKey> {
        self.host.as_ref().map(|host| &host.host().key)
    }
    pub(crate) fn container(&self) -> bool {
        self.host.as_ref().is_some_and(|host| host.container())
    }
    pub(crate) fn label(&self) -> Option<String> {
        self.host.as_ref().map(WatchHost::label)
    }

    /// What the current host looked like at the last reading.
    pub(crate) fn sampler(&self) -> Option<&Sampler> {
        let key = self.host.as_ref()?.key();
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

    fn follow(&mut self, source: MonitorSource, cx: &mut Context<Self>) {
        self.source = source;
        self.refresh(cx);
    }

    /// Starts, restarts or stops the watch to match the session, the
    /// settings and the details.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings(cx).clone();
        let host = watched_source(&self.source, self.local.as_ref(), &settings);
        let plan = Plan::new(&settings, self.open);
        let history = settings.history_points as usize;
        for record in &mut self.records {
            record.sampler.set_history_points(history);
        }

        let wanted = host.clone().zip(plan);
        let unchanged = match (&self.wanted, &wanted) {
            (Some((was, before)), Some((host, plan))) => was.same_as(host) && before == plan,
            (None, None) => false,
            _ => false,
        };
        if unchanged {
            return;
        }
        self.task = None;
        self.generation = self.generation.saturating_add(1);
        self.wanted = wanted;
        self.host = host;
        let Some((host, plan)) = self.wanted.clone() else {
            self.set_status(
                if settings.enabled
                    && plan.is_some()
                    && (!self.source.local() || !self.source.available())
                {
                    Status::Offline
                } else {
                    Status::Off
                },
                cx,
            );
            return;
        };
        self.remember(host.key(), history);
        if !host.host().connected {
            self.set_status(Status::Offline, cx);
            return;
        }
        let platform = match host.host().key {
            HostKey::Local => Some(local_platform()),
            HostKey::Remote { .. } => self.records[0].platform.clone(),
        };
        self.set_status(
            if platform.is_some() {
                Status::Starting
            } else {
                Status::Detecting
            },
            cx,
        );
        if self.generation == u64::MAX {
            self.set_status(Status::Failed("Monitor generation exhausted".into()), cx);
            return;
        }
        let generation = self.generation;
        self.task = Some(cx.spawn(async move |this, cx| {
            watch(this, host, platform, plan, generation, cx).await;
        }));
    }

    /// Moves the record of `key` to the front, creating it if need be.
    fn remember(&mut self, key: &MonitorKey, history: usize) {
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
    fn current(&mut self, key: &MonitorKey, generation: u64) -> Option<&mut Record> {
        if self.generation != generation {
            return None;
        }
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
    host: WatchHost,
    mut platform: Option<Platform>,
    plan: Plan,
    generation: u64,
    cx: &mut AsyncApp,
) {
    let key = host.key().clone();
    loop {
        let outcome = run(&this, &host, &mut platform, plan, generation, cx).await;
        let retry = this.update(cx, |this, cx| {
            if this.current(&key, generation).is_none() {
                return false;
            }
            let status = match outcome {
                Err(ExecError::Unsupported) => {
                    let name = match &platform {
                        Some(Platform::Unsupported(name)) => name.clone(),
                        Some(Platform::LinuxContainer) => "container cgroup accounting".into(),
                        _ => "unknown".into(),
                    };
                    this.set_status(Status::Unsupported(name), cx);
                    return false;
                }
                Err(ExecError::Disconnected) => "The connection closed.".into(),
                Err(ExecError::Failed(error)) => error,
                Err(error @ ExecError::NotFound(_)) => error.to_string(),
                Ok(()) => "The monitor script stopped.".into(),
            };
            if this.current(&key, generation).is_some() {
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
    host: &WatchHost,
    platform: &mut Option<Platform>,
    plan: Plan,
    generation: u64,
    cx: &mut AsyncApp,
) -> Result<(), ExecError> {
    let detected = match platform.clone() {
        Some(platform) => platform,
        None => {
            let detected = detect(host.host().exec.as_ref()).await?;
            let detected = if host.container() && detected == Platform::Linux {
                Platform::LinuxContainer
            } else if host.container() {
                Platform::Unsupported("container accounting requires Linux".into())
            } else {
                detected
            };
            *platform = Some(detected.clone());
            let key = host.key().clone();
            let remembered = detected.clone();
            let accepted = this
                .update(cx, |this, cx| {
                    if let Some(record) = this.current(&key, generation) {
                        record.platform = Some(remembered);
                        this.set_status(Status::Starting, cx);
                        true
                    } else {
                        false
                    }
                })
                .map_err(|_| ExecError::Disconnected)?;
            if !accepted {
                return Err(ExecError::Disconnected);
            }
            detected
        }
    };
    let mut watch = Watch::start(
        host.host().exec.as_ref(),
        detected,
        plan.metrics,
        plan.interval,
    )
    .await?;
    while let Some(reading) = watch.next().await {
        if host.container()
            && reading.container.as_ref().is_none_or(|counters| {
                counters.cpu_nanoseconds.is_none() && counters.memory_current.is_none()
            })
        {
            return Err(ExecError::Unsupported);
        }
        let key = host.key().clone();
        let applied = this.update(cx, |this, cx| {
            let Some(record) = this.current(&key, generation) else {
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
