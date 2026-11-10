//! One shared watch of each host's containers and images, shared by every window.
//!
//! Owns: [`ContainerHosts`], the registry that hands every caller asking about the same host
//! connection the same [`HostContainers`]; the engine found there, its last listings and
//! where listing stands; the one event stream followed per connection; and the rules for
//! when to list again: only for events that change a listing, only the listing they change,
//! once per burst, and every [`RETRY`] where there is no event stream.
//! Does not own: command lines, listing and event formats (`nocterm-containers`), host
//! execution or connections (`HostExec`), which host a window looks at, actions and views
//! (`nocterm-containers-ui`).
//! Ports used: `nocterm_session::HostExec`. Ports provided: [`ContainerHosts`] and
//! [`HostContainers`]. Settings section: none. Globals: none.
//! Invariants: a host connection, identified by its executor, has at most one watch, and so
//! one `events` process, however many windows show it; the watch lives while someone holds
//! its entity and stops its programs when the last one lets go; an event that changes no
//! listing runs no program.

mod host;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod watch;

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Weak},
};

use gpui::{App, AppContext as _, Entity, WeakEntity};
use nocterm_session::HostExec;

pub use host::{HostContainers, ListingStatus};
pub use watch::{RETRY, SETTLE};

/// The watched hosts, by connection. Cloning shares the registry.
#[derive(Clone, Default)]
pub struct ContainerHosts {
    watched: Rc<RefCell<Vec<Watched>>>,
}

struct Watched {
    exec: Weak<dyn HostExec>,
    host: WeakEntity<HostContainers>,
}

impl ContainerHosts {
    pub fn new() -> Self {
        Self::default()
    }

    /// The containers of the host reached through `exec`, watched once for everyone who asks
    /// about the same executor. A watch that stopped after a failure starts again.
    pub fn watch(&self, exec: &Arc<dyn HostExec>, cx: &mut App) -> Entity<HostContainers> {
        let found = {
            let mut watched = self.watched.borrow_mut();
            watched.retain(|entry| entry.host.upgrade().is_some());
            watched
                .iter()
                .filter(|entry| std::ptr::addr_eq(entry.exec.as_ptr(), Arc::as_ptr(exec)))
                .find_map(|entry| entry.host.upgrade())
        };
        if let Some(host) = found {
            host.update(cx, |host, cx| host.revive(cx));
            return host;
        }
        let host = cx.new(|cx| HostContainers::new(exec.clone(), cx));
        self.watched.borrow_mut().push(Watched {
            exec: Arc::downgrade(exec),
            host: host.downgrade(),
        });
        host
    }
}

#[cfg(test)]
mod tests;
