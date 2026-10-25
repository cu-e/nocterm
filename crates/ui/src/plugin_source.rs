//! Where the window shell and the editors read the plugins from.
//!
//! The system catalog is fixed for the life of the process, but the user plugins come and go.
//! Every reader therefore asks a [`PluginSource`] for the current [`PluginSet`] instead of
//! holding one, and observes the source to learn that a new set was published.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Entity, Subscription};
use nocterm_plugin_model::PluginSet;

/// The current plugins of both tiers.
pub trait PluginSource {
    /// The set in force now. The same `Arc` is returned until a new set is published, so a
    /// reader can compare two results by identity.
    fn current(&self, cx: &App) -> Arc<PluginSet>;

    /// Calls `notify` whenever a new set is published, until the subscription is dropped.
    fn observe(&self, notify: Box<dyn FnMut(&mut App)>, cx: &mut App) -> Subscription;
}

/// A source whose set never changes: the system catalog alone, or a set a test composes.
pub struct StaticPluginSource {
    set: Arc<PluginSet>,
}

impl StaticPluginSource {
    pub fn new(set: impl Into<Arc<PluginSet>>) -> Self {
        Self { set: set.into() }
    }
}

impl PluginSource for StaticPluginSource {
    fn current(&self, _: &App) -> Arc<PluginSet> {
        self.set.clone()
    }

    fn observe(&self, _: Box<dyn FnMut(&mut App)>, _: &mut App) -> Subscription {
        Subscription::new(|| {})
    }
}

struct Current {
    set: Arc<PluginSet>,
}

/// A source whose set the application replaces when the installed user plugins change.
#[derive(Clone)]
pub struct PluginSetHandle {
    current: Entity<Current>,
}

impl PluginSetHandle {
    pub fn new(set: PluginSet, cx: &mut App) -> Self {
        Self {
            current: cx.new(|_| Current { set: Arc::new(set) }),
        }
    }

    /// Publishes `set` to every observer. A set with the revision in force is ignored.
    pub fn replace(&self, set: PluginSet, cx: &mut App) {
        self.current.update(cx, |current, cx| {
            if current.set.revision() == set.revision() && current.set.users() == set.users() {
                return;
            }
            current.set = Arc::new(set);
            cx.notify();
        });
    }
}

impl PluginSource for PluginSetHandle {
    fn current(&self, cx: &App) -> Arc<PluginSet> {
        self.current.read(cx).set.clone()
    }

    fn observe(&self, mut notify: Box<dyn FnMut(&mut App)>, cx: &mut App) -> Subscription {
        cx.observe(&self.current, move |_, cx| notify(cx))
    }
}

#[cfg(test)]
mod tests;
