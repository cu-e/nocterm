//! Effects whose answer comes later: requests to the user's web address and timers.
//!
//! The broker admits them while it applies a call's effects: a request counts against the
//! plugin's requests in flight, a timer against its waiting timers (one per key). The host then
//! starts what was admitted; each answer comes back as an event to the contribution that asked,
//! through [`PluginHost::deliver`], and only while the plugin runs the package that asked.
//! Disabling, replacing, removing or faulting a plugin forgets its timers and stops counting
//! its requests, whose late answers are then dropped.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc, time::Duration};

use gpui::{Context, Task, WindowId};
use nocterm_plugin_api::{
    ContentHash, Event, HostError, PluginNet,
    capability::{
        net::{MAX_IN_FLIGHT, NetAnswer, NetCall},
        timers::MAX_TIMERS,
    },
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::PluginProviders;

use crate::{Call, PluginHost};

/// Who asked: the call whose event raised the effect, the package that answered it, and the
/// window's providers for the answer's own call.
pub(crate) struct Target {
    call: Call,
    hash: ContentHash,
    providers: PluginProviders,
}

impl Target {
    pub(crate) fn new(call: Call, hash: ContentHash, providers: PluginProviders) -> Self {
        Self {
            call,
            hash,
            providers,
        }
    }
}

enum Work {
    Request(NetCall),
    /// A request that cannot be sent, answered at once with status 0.
    Answered(NetAnswer),
    Timer(Duration),
}

struct Admitted {
    target: Target,
    key: String,
    work: Work,
}

/// The requests in flight of each plugin, under the epoch they were sent in.
#[derive(Default)]
struct InFlight {
    counts: BTreeMap<PluginId, (u64, usize)>,
    epochs: BTreeMap<PluginId, u64>,
}

impl InFlight {
    fn epoch(&self, plugin: &PluginId) -> u64 {
        self.epochs.get(plugin).copied().unwrap_or_default()
    }

    fn count(&self, plugin: &PluginId) -> usize {
        match self.counts.get(plugin) {
            Some((epoch, count)) if *epoch == self.epoch(plugin) => *count,
            _ => 0,
        }
    }

    fn add(&mut self, plugin: &PluginId) -> u64 {
        let epoch = self.epoch(plugin);
        let count = self.count(plugin);
        self.counts.insert(plugin.clone(), (epoch, count + 1));
        epoch
    }

    fn done(&mut self, plugin: &PluginId, epoch: u64) {
        if let Some((current, count)) = self.counts.get_mut(plugin)
            && *current == epoch
        {
            *count = count.saturating_sub(1);
        }
    }

    fn forget(&mut self, plugin: &PluginId) {
        *self.epochs.entry(plugin.clone()).or_default() += 1;
        self.counts.remove(plugin);
    }
}

/// The waiting timers by plugin and key, each with the window it fires in.
type Timers = BTreeMap<(PluginId, String), (WindowId, u64, Task<()>)>;

pub(crate) struct Deferred {
    net: Option<Arc<dyn PluginNet>>,
    admitted: Vec<Admitted>,
    in_flight: Rc<RefCell<InFlight>>,
    timers: Rc<RefCell<Timers>>,
    next_timer: u64,
}

impl Deferred {
    pub(crate) fn new(net: Option<Arc<dyn PluginNet>>) -> Self {
        Self {
            net,
            admitted: Vec::new(),
            in_flight: Rc::default(),
            timers: Rc::default(),
            next_timer: 0,
        }
    }

    /// Admits a request, or the answer that says why it cannot be sent. `rate-limited` while
    /// the plugin has [`MAX_IN_FLIGHT`] requests in flight, `unavailable` without a network port.
    pub(crate) fn admit_request(
        &mut self,
        target: Target,
        key: String,
        built: Result<NetCall, NetAnswer>,
    ) -> Result<(), HostError> {
        if self.net.is_none() {
            return Err(HostError::Unavailable);
        }
        let waiting = self
            .admitted
            .iter()
            .filter(|item| {
                item.target.call.plugin == target.call.plugin
                    && matches!(item.work, Work::Request(_))
            })
            .count();
        if self.in_flight.borrow().count(&target.call.plugin) + waiting >= MAX_IN_FLIGHT {
            return Err(HostError::RateLimited {
                retry_after_ms: 1000,
            });
        }
        let work = match built {
            Ok(call) => Work::Request(call),
            Err(answer) => Work::Answered(answer),
        };
        self.admitted.push(Admitted { target, key, work });
        Ok(())
    }

    /// Admits a timer. Setting a key that waits replaces it; a new key past [`MAX_TIMERS`] is
    /// `rate-limited`.
    pub(crate) fn admit_timer(
        &mut self,
        target: Target,
        key: String,
        after_ms: u32,
    ) -> Result<(), HostError> {
        let plugin = &target.call.plugin;
        let timers = self.timers.borrow();
        let replaces = timers.contains_key(&(plugin.clone(), key.clone()));
        let waiting = timers.keys().filter(|(owner, _)| owner == plugin).count();
        drop(timers);
        if !replaces && waiting >= MAX_TIMERS {
            return Err(HostError::RateLimited {
                retry_after_ms: 1000,
            });
        }
        self.admitted.push(Admitted {
            target,
            key,
            work: Work::Timer(Duration::from_millis(u64::from(after_ms))),
        });
        Ok(())
    }

    /// Starts every admitted request and timer.
    pub(crate) fn start(&mut self, cx: &mut Context<PluginHost>) {
        for item in std::mem::take(&mut self.admitted) {
            match item.work {
                Work::Request(call) => self.send(item.target, item.key, call, cx),
                Work::Answered(answer) => answer_later(item.target, item.key, answer, cx),
                Work::Timer(after) => self.wait(item.target, item.key, after, cx),
            }
        }
    }

    fn send(&self, target: Target, key: String, call: NetCall, cx: &mut Context<PluginHost>) {
        let Some(net) = self.net.clone() else {
            return;
        };
        let plugin = target.call.plugin.clone();
        let epoch = self.in_flight.borrow_mut().add(&plugin);
        let in_flight = self.in_flight.clone();
        let answer = cx
            .background_executor()
            .spawn(async move { net.send(&call) });
        cx.spawn(async move |host, cx| {
            let answer = answer.await;
            in_flight.borrow_mut().done(&plugin, epoch);
            if in_flight.borrow().epoch(&plugin) != epoch {
                return;
            }
            let _ = host.update(cx, |host, cx| {
                deliver(host, &target, response(key, answer), cx);
            });
        })
        .detach();
    }

    fn wait(&mut self, target: Target, key: String, after: Duration, cx: &mut Context<PluginHost>) {
        self.next_timer += 1;
        let id = self.next_timer;
        let slot = (target.call.plugin.clone(), key.clone());
        let timers = self.timers.clone();
        let window = target.call.window;
        let fired = slot.clone();
        let task = cx.spawn(async move |host, cx| {
            cx.background_executor().timer(after).await;
            let current = timers
                .borrow()
                .get(&fired)
                .is_some_and(|(_, waiting, _)| *waiting == id);
            if !current {
                return;
            }
            timers.borrow_mut().remove(&fired);
            let _ = host.update(cx, |host, cx| {
                deliver(host, &target, Event::Timer { key }, cx);
            });
        });
        self.timers.borrow_mut().insert(slot, (window, id, task));
    }

    /// The plugin stopped: its timers go and its requests in flight no longer answer.
    pub(crate) fn forget(&mut self, plugin: &PluginId) {
        self.admitted
            .retain(|item| item.target.call.plugin != *plugin);
        self.timers
            .borrow_mut()
            .retain(|(owner, _), _| owner != plugin);
        self.in_flight.borrow_mut().forget(plugin);
    }

    /// A window closed: the timers that would wake its contributions go.
    pub(crate) fn release_window(&mut self, window: WindowId) {
        self.admitted
            .retain(|item| item.target.call.window != window);
        self.timers
            .borrow_mut()
            .retain(|_, (fires_in, _, _)| *fires_in != window);
    }
}

fn answer_later(target: Target, key: String, answer: NetAnswer, cx: &mut Context<PluginHost>) {
    cx.spawn(async move |host, cx| {
        let _ = host.update(cx, |host, cx| {
            deliver(host, &target, response(key, answer), cx);
        });
    })
    .detach();
}

fn response(key: String, answer: NetAnswer) -> Event {
    Event::Response {
        key,
        status: answer.status,
        body: answer.body,
    }
}

fn deliver(host: &mut PluginHost, target: &Target, event: Event, cx: &mut Context<PluginHost>) {
    let delivered = host.deliver(
        &target.call,
        Some(&target.hash),
        event,
        &target.providers,
        cx,
    );
    if let Err(error) = delivered {
        tracing::debug!(plugin = %target.call.plugin, %error, "a deferred plugin event was dropped");
    }
}
