//! The host of user plugins: installed plugins, grants, the effect broker, rate limits,
//! cadence and caps, and the two storage ports.
//!
//! A plugin is a function from a [`Snapshot`](nocterm_plugin_api::Snapshot) to a view and from
//! an event to at most eight effects. [`PluginHost`] assembles the snapshot, runs the guest
//! through the [`PluginRuntime`](nocterm_plugin_api::PluginRuntime) port, validates the view,
//! and applies the effects after the current update ends.
//!
//! Owns: the registry of installed plugins and their grants (bound to the sha256 the user
//! accepted), the effect broker with its queue, gesture tokens, per-capability rate limits and
//! apply-time re-check, the sampling cadence and render budget, fault counting, the caps of
//! design 5.12, the `PluginRecords` and `PluginData` ports and their documents, and the
//! handling of the storage effects.
//! Does not own: any provider or the `Observe`/`Apply` traits (`nocterm-plugin-ports`), the
//! packages on disk (`PluginPackages`), a runtime, views, consent dialogs, settings forms or any
//! settings section.
//! Ports used: `PluginPackages`, `PluginRuntime`, `PluginRecords`, `PluginData`, and the window's
//! `PluginProviders`, which arrive with each call. Provided: none.
//! Settings section: none. Globals: none. The host is an entity composed by the application,
//! never looked up.
//! Invariants: an effect is bound to `(plugin, sha256, window, contribution)` when queued and
//! checked again when applied (grant, enabled, own contribution, gesture, rate, surface);
//! disabling, replacing, removing or faulting a plugin drops what is queued for it; a gesture
//! token is single-use, valid for one second and minted only by a pointer or an unrepeated
//! Enter or Space; observations are sampled no faster than 1 Hz (4 Hz for a panel with focus);
//! at most 16 plugins are enabled and 32 renders happen per second; three traps or missed
//! deadlines in a session disable a plugin; a broken data file gives defaults and is replaced
//! only by the user's next change.

mod broker;
mod cadence;
mod caps;
mod data;
mod error;
mod faults;
mod gesture;
mod host;
mod rate;
mod records;
mod registry;
mod snapshot;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use caps::{
    GESTURE_TTL, MAX_ENABLED, MAX_FAULTS, MAX_OUTCOMES, MAX_RENDERS_PER_SECOND, OBSERVE_EVERY,
    OBSERVE_EVERY_FOCUSED,
};
pub use data::{MAX_VALUES_BYTES, PluginData, PluginDataChange, PluginDataDoc, StoredValue};
pub use error::{
    BrokenPackage, CallError, DataBoundsError, DocumentError, ManageError, PluginDataError,
    RecordsError,
};
pub use gesture::Input;
pub use host::{Call, HostPorts, PluginHost};
pub use records::{
    AuditAction, AuditEntry, MAX_AUDIT, MAX_RECORDS, PluginRecord, PluginRecords, RecordsDoc,
};
pub use registry::{PluginStatus, PluginSummary};

#[cfg(test)]
mod tests;
