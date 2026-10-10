//! Ordered persistence and bounded shutdown for application state.
//!
//! Owns: settings documents, snapshot admission, write ordering and quit capture.
//! Does not own: owner-defined settings sections, views or custom snapshot codecs.
//! Ports used: `SnapshotWriter`. Ports provided: `ShutdownCoordinator`.
//! Settings section: none. Globals: none.
//! Invariants: final work is immutable before App teardown; one disk gate orders
//! normal and final writes; durable state advances only after successful writes.

mod captured;
mod error;
mod persisted;
mod settings;
mod shutdown;

pub use captured::{Captured, CapturedFeed, CapturedOutcome, CommitPhase, Committed};
pub use error::StoreError;
pub use persisted::{Persisted, Publication, Receipt, Revision, SnapshotWriter};
pub use settings::{SETTINGS_CAPACITY, SettingsConfig, SettingsHandle};
pub use shutdown::{SHUTDOWN_BUDGET, ShutdownCoordinator, ShutdownReport};
