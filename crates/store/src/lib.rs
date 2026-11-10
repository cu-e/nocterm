//! Ordered persistence and bounded shutdown for application state.
//!
//! Owns: settings documents, snapshot/keyed admission, common normal/captured disk
//! execution, physical publication stamps and synchronous quit capture.
//! Does not own: owner-defined settings sections, views or custom snapshot codecs.
//! Ports used: `SnapshotWriter`, `KeyedWriter`. Ports provided: `ShutdownCoordinator`.
//! Settings section: none. Globals: none.
//! Invariants: final work is immutable before App teardown; one disk gate orders
//! normal and final writes; durable state advances only after successful writes;
//! ready-key capacity bounds dispatch tokens and never drops accepted dirty keys.
//! A borrowed retained-values observer includes latest and older active generations;
//! consumers may account payload bytes without duplicating the keyed ownership map.
//! Checked settings edits reject their whole ordered candidate before revision/publication.
//! Edit/backend causes retain their concrete errors through core shared ownership.
//! Joined failures preserve their original primary cause and separate secondary diagnostic.

mod append;
mod captured;
mod error;
mod keyed;
mod persisted;
mod settings;
mod shutdown;
mod worker;

pub use append::{AppendHandle, AppendPersisted, AppendStatus, CapturedAppend, RecordingRegistry};
pub use captured::{Captured, CapturedFeed, CapturedOutcome, CommitPhase, Committed};
pub use error::StoreError;
pub use persisted::{Persisted, Publication, Receipt, Revision, SnapshotWriter};
pub use settings::{SETTINGS_CAPACITY, SettingsConfig, SettingsHandle};
pub use shutdown::{SHUTDOWN_BUDGET, ShutdownCoordinator, ShutdownReport};

pub use keyed::{
    CapturedKeys, KeyedOutcome, KeyedPersisted, KeyedRetention, KeyedStamp, KeyedWriter,
};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
