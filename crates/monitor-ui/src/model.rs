//! Selected surface resource identity and one generation-bound monitoring watch.
//!
//! General contexts retain explicitly supplied local policy. Project contexts
//! observe only their original inner session; unavailable resources never fall
//! back to an outer or local executor. History is bounded per surface/resource.

use nocterm_monitor_service::{MonitorWatch, MonitorWatches, WatchPlan, WatchStatus};
use std::{sync::Arc, time::Duration};

use crate::MonitorSettings;
#[cfg(test)]
pub(crate) use crate::source::watched_host;
use crate::source::{MonitorKey, WatchHost, watched_source};
use gpui_kit::{App, Context, Entity, EventEmitter, Subscription};
use nocterm_monitor::{MetricSet, Platform, Sampler, local_platform};
use nocterm_session::HostExec;
use nocterm_workspace::{HostKey, MonitorSource, Workspace, WorkspaceEvent};

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
    sampler: Sampler,
    platform: Option<(std::sync::Weak<dyn HostExec>, Platform)>,
}

pub(crate) struct HostMonitor {
    consumer: gpui_kit::EntityId,
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
    collectors: MonitorWatches,
    watch: Option<CollectedWatch>,
    generation: u64,
    _subscriptions: Vec<Subscription>,
}

/// The model's retained subscription and last applied raw reading.
struct CollectedWatch {
    collection: Entity<MonitorWatch>,
    sequence: u64,
    _observation: Subscription,
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
            consumer: cx.entity_id(),
            local: installed.local(),
            settings: installed.settings().clone(),
            source,
            host: None,
            open: false,
            status: Status::Off,
            records: Vec::new(),
            wanted: None,
            collectors: installed.collectors().clone(),
            watch: None,
            generation: 0,
            _subscriptions: vec![
                follow,
                observation,
                cx.on_release(|this, cx| this.stop_watch(cx)),
            ],
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
        self.generation = self.generation.saturating_add(1);
        self.wanted = wanted;
        self.host = host;
        let Some((host, plan)) = self.wanted.clone() else {
            self.stop_watch(cx);
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
            self.stop_watch(cx);
            self.set_status(Status::Offline, cx);
            return;
        }
        if self.generation == u64::MAX {
            self.stop_watch(cx);
            self.set_status(Status::Failed("Monitor generation exhausted".into()), cx);
            return;
        }
        self.attach(host, plan, cx);
    }

    fn attach(&mut self, host: WatchHost, plan: Plan, cx: &mut Context<Self>) {
        let platform = if matches!(host.host().key, HostKey::Local) {
            Some(local_platform())
        } else {
            self.records[0]
                .platform
                .as_ref()
                .filter(|(exec, _)| {
                    std::ptr::addr_eq(exec.as_ptr(), Arc::as_ptr(&host.host().exec))
                })
                .map(|(_, platform)| platform.clone())
        };
        let collection =
            self.collectors
                .watch(host.host().exec.clone(), host.container(), platform, cx);
        let old = self.watch.take();
        let sequence = old
            .as_ref()
            .filter(|old| old.collection == collection)
            .map_or(0, |old| old.sequence);
        if let Some(old) = old
            && old.collection != collection
        {
            old.collection
                .update(cx, |watch, cx| watch.unsubscribe(self.consumer, cx));
        }
        let generation = self.generation;
        let observation = cx.observe(&collection, move |this, collection, cx| {
            this.apply_collection(&collection, generation, cx)
        });
        let consumer = cx.entity_id();
        collection.update(cx, |watch, cx| {
            watch.subscribe(
                consumer,
                WatchPlan {
                    metrics: plan.metrics,
                    interval: plan.interval,
                },
                cx,
            )
        });
        self.watch = Some(CollectedWatch {
            collection: collection.clone(),
            sequence,
            _observation: observation,
        });
        self.apply_collection(&collection, generation, cx);
    }

    fn stop_watch(&mut self, cx: &mut App) {
        if let Some(watch) = self.watch.take() {
            let consumer = self.consumer;
            watch
                .collection
                .update(cx, |watch, cx| watch.unsubscribe(consumer, cx));
        }
    }

    fn apply_collection(
        &mut self,
        collection: &Entity<MonitorWatch>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation {
            return;
        }
        let Some(watch) = self
            .watch
            .as_mut()
            .filter(|watch| watch.collection == *collection)
        else {
            return;
        };
        let collected = collection.read(cx);
        if let Some(platform) = collected.platform()
            && let (Some(host), Some(record)) = (&self.host, self.records.first_mut())
        {
            record.platform = Some((Arc::downgrade(&host.host().exec), platform.clone()));
        }
        let status = match collected.status() {
            WatchStatus::Off => Status::Off,
            WatchStatus::Detecting => Status::Detecting,
            WatchStatus::Starting => Status::Starting,
            WatchStatus::Running => Status::Running,
            WatchStatus::Unsupported(name) => Status::Unsupported(name.clone()),
            WatchStatus::Failed(error) => Status::Failed(error.to_string()),
        };
        let reading = collected
            .reading()
            .filter(|(sequence, _)| *sequence != watch.sequence)
            .map(|(sequence, reading)| (sequence, reading.clone()));
        if let Some((sequence, reading)) = reading {
            watch.sequence = sequence;
            if let Some(record) = self.records.first_mut() {
                record.sampler.apply(reading);
                cx.emit(Changed);
                cx.notify();
            }
        }
        self.set_status(status, cx);
    }

    /// Moves the record of `key` to the front, creating it if need be.
    fn remember(&mut self, key: &MonitorKey, history: usize) {
        let record = match self.records.iter().position(|record| record.key == *key) {
            Some(index) => self.records.remove(index),
            None => Record {
                key: key.clone(),
                sampler: Sampler::new(history),
                platform: None,
            },
        };
        self.records.insert(0, record);
        self.records.truncate(REMEMBERED_HOSTS);
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.emit(Changed);
            cx.notify();
        }
    }
}
