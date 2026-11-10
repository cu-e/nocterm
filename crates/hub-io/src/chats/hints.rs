//! Hints that a hub's chat list changed on this device: a refreshed list that differs from the
//! kept one. Watchers of the chat list hear them beside the hub's own changes.

use tokio::sync::watch;

pub(crate) struct ChatHints(watch::Sender<u64>);

impl Default for ChatHints {
    fn default() -> Self {
        Self(watch::channel(0).0)
    }
}

impl ChatHints {
    pub(crate) fn raise(&self) {
        self.0.send_modify(|count| *count = count.wrapping_add(1));
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}
