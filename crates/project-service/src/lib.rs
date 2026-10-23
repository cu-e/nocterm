//! Retained workspace catalog, original window claims and setup admission.
//!
//! Owns: durable checked catalog edits, source generations and duplicate-window
//! focus requests, captured setup lifecycle, prepared-resource save retry and
//! cancellation/cleanup admission and non-destructive environment job admission.
//! Does not own: native sessions, processes,
//! filesystem implementations or views.
//! Ports used: supplied SnapshotWriter, ProjectMetadataSource, shared Persisted
//! and ShutdownCoordinator, captured ProjectPreparation and HostExec.
//! Ports provided: ProjectsHandle, WorkspaceSetup, EnvironmentOperation and RemovalOperation.
//! Settings section: none. Globals: none.
//! Invariants: failed writes retain the committed catalog; stale original
//! definition/window receipts never retarget a pending connection action; setup
//! shutdown freezes admission and cleanup never deletes attached resources or a
//! resource whose catalog publication succeeded. Environment jobs retain exact captured
//! authority, serialize install requests per account/environment independently of folder
//! and profile alias, and drain on shared shutdown without catalog or deletion effects. Explicit removal jobs
//! serialize exact resource actions, retain acknowledged effects and only then publish a
//! checked catalog removal; failed publication retries never repeat the resource action.

mod catalog;
mod claims;
mod environment;
mod error;
pub use environment::EnvironmentOperation;
mod installation;
mod prepared;
mod removal;
pub use removal::{RemovalContainerAction, RemovalOperation};
mod setup;
pub use setup::WorkspaceSetup;

pub use catalog::ProjectBinding;
pub use claims::{ClaimOutcome, ProjectClaim};
pub use error::{ProjectsError, WorkspaceRemovalError};
pub use installation::ProjectsHandle;
pub use prepared::PreparedProjects;
