//! Prepares connected-server metadata off the GUI thread, then publishes only to its
//! original current session before asking hubs to reconcile their SSH trust records.
use gpui_kit::{App, BackgroundExecutor, Entity, Task};
use nocterm_connections::HostObserver;
use nocterm_connections_store::ProbeSourceLease;
use nocterm_hubs::Hubs;
use nocterm_session::RemoteFs;
use nocterm_ssh_keys::authorized_keys::{ConnectedHost, ConnectedHosts};
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{Arc, Weak},
};

type Reconcile = Rc<dyn Fn(&str, &mut App)>;
struct Announcement {
    host: ConnectedHost,
    source: ProbeSourceLease,
}
struct Active {
    connection: String,
    fs: Weak<dyn RemoteFs>,
    source: ProbeSourceLease,
    serial: u64,
    _task: Task<()>,
}
#[derive(Default)]
struct PreparationQueue {
    waiting: VecDeque<Announcement>,
    active: Option<Active>,
    serial: u64,
}
struct Reconciling {
    sessions: Arc<dyn ConnectedHosts>,
    reconcile: Reconcile,
    executor: BackgroundExecutor,
    queue: Rc<RefCell<PreparationQueue>>,
}
impl Reconciling {
    fn new(
        sessions: Arc<dyn ConnectedHosts>,
        reconcile: Reconcile,
        executor: BackgroundExecutor,
    ) -> Self {
        Self {
            sessions,
            reconcile,
            executor,
            queue: Rc::default(),
        }
    }
}
impl HostObserver for Reconciling {
    fn connected(&self, host: ConnectedHost, source: ProbeSourceLease, cx: &mut App) {
        if !source.is_current(cx) {
            return;
        }
        {
            let mut queue = self.queue.borrow_mut();
            queue
                .waiting
                .retain(|request| request.source.is_current(cx));
            if queue.active.as_ref().is_some_and(|active| {
                active.connection == host.connection
                    && active.source.same_origin(&source)
                    && std::ptr::addr_eq(active.fs.as_ptr(), Arc::as_ptr(&host.fs))
            }) {
                return;
            }
            let announcement = Announcement { host, source };
            if let Some(index) = queue
                .waiting
                .iter()
                .position(|request| request.host.connection == announcement.host.connection)
            {
                queue.waiting[index] = announcement;
            } else {
                queue.waiting.push_back(announcement);
            }
        }
        begin(
            &self.queue,
            self.sessions.clone(),
            self.reconcile.clone(),
            self.executor.clone(),
            cx,
        );
    }
}

impl PreparationQueue {
    fn take_next(&mut self, cx: &App) -> Option<(Announcement, u64)> {
        if self.active.is_some() {
            return None;
        }
        self.waiting.retain(|request| request.source.is_current(cx));
        let request = self.waiting.pop_front()?;
        self.serial = self.serial.checked_add(1)?;
        Some((request, self.serial))
    }
    fn accept(
        &mut self,
        prepared: &ConnectedHost,
        serial: u64,
        expected: &(String, String, u16),
        cx: &App,
    ) -> bool {
        self.waiting.retain(|request| request.source.is_current(cx));
        let newer = self
            .waiting
            .iter()
            .any(|request| request.host.connection == prepared.connection);
        let current = self.active.as_ref().is_some_and(|active| {
            active.serial == serial
                && active.connection == prepared.connection
                && active.source.is_current(cx)
                && std::ptr::addr_eq(active.fs.as_ptr(), Arc::as_ptr(&prepared.fs))
                && prepared.identity.user == expected.0
                && prepared.identity.host == expected.1
                && prepared.port == expected.2
        });
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.serial == serial)
        {
            self.active = None;
        }
        current && !newer
    }
}

/// One native prepare at a time. Pending work is replaced by the latest announcement
/// per live saved connection; obsolete receipts never publish state from a worker.
fn begin(
    queue: &Rc<RefCell<PreparationQueue>>,
    sessions: Arc<dyn ConnectedHosts>,
    reconcile: Reconcile,
    executor: BackgroundExecutor,
    cx: &mut App,
) {
    let Some((Announcement { host, source }, serial)) = queue.borrow_mut().take_next(cx) else {
        return;
    };
    let connection = host.connection.clone();
    let fs = Arc::downgrade(&host.fs);
    let expected = (
        host.identity.user.clone(),
        host.identity.host.clone(),
        host.port,
    );
    let weak = Rc::downgrade(queue);
    let native = sessions.clone();
    let next_executor = executor.clone();
    let preparation = executor.spawn(async move { native.prepare(host) });
    let task = cx.spawn(async move |cx| {
        let prepared = preparation.await;
        cx.update(|cx| {
            let Some(queue) = weak.upgrade() else {
                return;
            };
            let accepted = queue.borrow_mut().accept(&prepared, serial, &expected, cx);
            if accepted {
                let connection = prepared.connection.clone();
                sessions.connected(prepared);
                reconcile(&connection, cx);
            }
            begin(&queue, sessions, reconcile, next_executor, cx);
        });
    });
    queue.borrow_mut().active = Some(Active {
        connection,
        fs,
        source,
        serial,
        _task: task,
    });
}

/// The installation retains the observer and the original executor through its work.
pub(crate) fn observe(
    sessions: Arc<dyn ConnectedHosts>,
    hubs: Entity<Hubs>,
    connections: &nocterm_connections::Installed,
    executor: BackgroundExecutor,
) {
    let reconcile = Rc::new(move |connection: &str, cx: &mut App| {
        hubs.update(cx, |hubs, cx| hubs.reconcile_agent_host(connection, cx));
    });
    connections.bind_host_observer(Rc::new(Reconciling::new(sessions, reconcile, executor)));
}

#[cfg(test)]
mod tests;
