//! Retained folder catalog publication and original window claims.
//!
//! Owns: durable checked catalog edits, source generations and duplicate-window
//! focus requests. Does not own: sessions, processes, filesystem I/O or views.
//! Ports used: supplied SnapshotWriter, ProjectMetadataSource, shared Persisted
//! and ShutdownCoordinator. Ports provided: ProjectsHandle.
//! Settings section: none. Globals: none.
//! Invariants: failed writes retain the committed catalog; stale original
//! definition/window receipts never retarget a pending connection action.

mod catalog;
mod claims;
mod error;
mod installation;
mod prepared;

pub use catalog::ProjectBinding;
pub use claims::{ClaimOutcome, ProjectClaim};
pub use error::ProjectsError;
pub use installation::ProjectsHandle;
pub use prepared::PreparedProjects;
