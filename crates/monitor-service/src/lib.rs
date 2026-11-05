//! Raw resource collection shared by consumers of the same captured executor.
//! Owns: one watch per executor/scope, the metric union and fastest requested cadence,
//! typed watch status, one latest reading, and cancellation when the last consumer leaves.
//! Does not own: selected windows/surfaces, presentation histories, views or native processes.
//! Ports used: HostExec. Ports provided: MonitorWatches, MonitorWatch. Settings: none. Globals: none.
//! Invariants: descriptive host names never merge authority; subscriptions retain their original
//! executor; obsolete plans/results cannot update the current watch generation.
mod watch;
use gpui::{App, AppContext as _, Context, Entity, EntityId, Task, WeakEntity};
use nocterm_monitor::{MetricSet, Platform, Reading};
use nocterm_session::{ExecError, HostExec};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Weak},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchPlan {
    pub metrics: MetricSet,
    pub interval: Duration,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchStatus {
    Off,
    Detecting,
    Starting,
    Running,
    Unsupported(String),
    Failed(ExecError),
}
#[derive(Clone, Default)]
pub struct MonitorWatches {
    watched: Rc<RefCell<Vec<Watched>>>,
}
struct Watched {
    exec: Weak<dyn HostExec>,
    container: bool,
    watch: WeakEntity<MonitorWatch>,
}
impl MonitorWatches {
    pub fn watch(
        &self,
        exec: Arc<dyn HostExec>,
        container: bool,
        platform: Option<Platform>,
        cx: &mut App,
    ) -> Entity<MonitorWatch> {
        let mut watched = self.watched.borrow_mut();
        watched.retain(|entry| entry.watch.upgrade().is_some());
        if let Some(watch) = watched
            .iter()
            .filter(|entry| {
                entry.container == container
                    && std::ptr::addr_eq(entry.exec.as_ptr(), Arc::as_ptr(&exec))
            })
            .find_map(|entry| entry.watch.upgrade())
        {
            return watch;
        }
        let watch = cx.new(|_| MonitorWatch {
            exec: exec.clone(),
            container,
            platform: platform.map(|platform| scoped_platform(platform, container)),
            plans: Default::default(),
            plan: None,
            status: WatchStatus::Off,
            reading: None,
            sequence: 0,
            generation: 0,
            task: None,
        });
        watched.push(Watched {
            exec: Arc::downgrade(&exec),
            container,
            watch: watch.downgrade(),
        });
        watch
    }
}
pub struct MonitorWatch {
    exec: Arc<dyn HostExec>,
    container: bool,
    platform: Option<Platform>,
    plans: std::collections::BTreeMap<EntityId, WatchPlan>,
    plan: Option<WatchPlan>,
    status: WatchStatus,
    reading: Option<Reading>,
    sequence: u64,
    generation: u64,
    task: Option<Task<()>>,
}
impl MonitorWatch {
    pub fn platform(&self) -> Option<&Platform> {
        self.platform.as_ref()
    }
    pub fn status(&self) -> &WatchStatus {
        &self.status
    }
    pub fn reading(&self) -> Option<(u64, &Reading)> {
        self.reading
            .as_ref()
            .map(|reading| (self.sequence, reading))
    }
    pub fn subscribe(&mut self, consumer: EntityId, plan: WatchPlan, cx: &mut Context<Self>) {
        if plan.metrics.is_empty() {
            self.plans.remove(&consumer);
        } else {
            self.plans.insert(
                consumer,
                WatchPlan {
                    interval: plan.interval.max(Duration::from_secs(1)),
                    ..plan
                },
            );
        }
        self.refresh(cx);
    }
    pub fn unsubscribe(&mut self, consumer: EntityId, cx: &mut Context<Self>) {
        self.plans.remove(&consumer);
        self.refresh(cx);
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let plan = self
            .plans
            .values()
            .copied()
            .reduce(|left, right| WatchPlan {
                metrics: left.metrics.union(right.metrics),
                interval: left.interval.min(right.interval),
            });
        if self.plan == plan {
            return;
        }
        self.task = None;
        self.generation = self.generation.saturating_add(1);
        self.plan = plan;
        let Some(plan) = plan else {
            self.set_status(WatchStatus::Off, cx);
            return;
        };
        if self.generation == u64::MAX {
            self.set_status(
                WatchStatus::Failed(ExecError::Failed("Monitor generation exhausted".into())),
                cx,
            );
            return;
        }
        let generation = self.generation;
        let exec = self.exec.clone();
        let container = self.container;
        let platform = self.platform.clone();
        self.set_status(
            if platform.is_some() {
                WatchStatus::Starting
            } else {
                WatchStatus::Detecting
            },
            cx,
        );
        self.task = Some(cx.spawn(async move |this, cx| {
            watch::watch(this, exec, container, platform, plan, generation, cx).await
        }));
    }
    fn set_status(&mut self, status: WatchStatus, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }
    fn accept(&mut self, generation: u64, reading: Reading, cx: &mut Context<Self>) -> bool {
        if generation != self.generation || self.plan.is_none() {
            return false;
        }
        self.sequence = self.sequence.saturating_add(1);
        self.reading = Some(reading);
        self.status = WatchStatus::Running;
        cx.notify();
        true
    }
}

fn scoped_platform(platform: Platform, container: bool) -> Platform {
    if !container {
        platform
    } else if platform == Platform::Linux || platform == Platform::LinuxContainer {
        Platform::LinuxContainer
    } else {
        Platform::Unsupported("container accounting requires Linux".into())
    }
}

#[cfg(test)]
mod tests;
