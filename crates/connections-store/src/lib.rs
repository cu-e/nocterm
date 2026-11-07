//! Ordered connection publication and bounded raw server-fact acquisition.
//! Owns: checked profiles/recents, committed publication carrier, raw Known/flag
//! state, original-source tickets, policy history, physical reservations and capture,
//! and the synchronization of profiles with a replica (ProfileReplication), including the rollback of a refused change.
//! Does not own: native I/O, views/images, session providers, settings storage,
//! credentials, terminal implementation or observation composition.
//! Ports used: supplied writers/clock, DNS/country/flag/OS observation contracts,
//! CountryPolicySource, ProbeSourceAuthority, BackgroundExecutor, ShutdownCoordinator,
//! ProfileReplica (supplied by an adapter) and a supplied Persisted replica state.
//! Ports provided: ConnectionStore, ProfilePublications, FactsService, ProfileReplication.
//! Settings: supplied enabled/revision policy projection. Globals: none.
//! Invariants: committed carrier and current original source/policy are read at
//! admission/completion; stale jobs keep their physical slot until actual finish;
//! 32 slots are shared across windows and paired reservations are atomic. Facts
//! use common optimistic persistence/capture and its existing 180-ms deadline.
//! Replication writes local profiles only through ConnectionStore, never echoes an
//! applied remote change, leaves a profile the replica holds unreadably untouched,
//! reconciles nothing while the local document failed to load and repeats a pass
//! that met a concurrent change at most three times.
//! Read-only diagnostics preserve their original allocation and concrete sources through core SharedError.
mod connections;
mod error;
mod prepared;
mod publication;
mod replication;
pub use connections::ConnectionStore;
pub use error::ConnectionsError;
pub use nocterm_connections_model::profile_authority::{
    ProfileAuthorityRow, ProfilePublication, ProfileStamp,
};
pub use publication::projection::ProfilePublications;
pub use replication::{
    ProfileReplication, ReplicationEvent, ReplicationFailure, ReplicationFeedError,
    ReplicationStatus,
};

mod facts;
mod observation;
pub use facts::{
    CountryPolicySnapshot, CountryPolicySource, FactWorkload, FactsAdmissionError, FactsEvent,
    FactsPrepared, FactsService, FlagState, MAX_FACT_JOBS, ProbeOutcome, ProbeRequest,
    ProbeSourceAuthority, ProbeSourceIdentity, ProbeSourceLease,
};
pub use observation::ObservationDeps;

#[cfg(feature = "test-support")]
pub mod test_support;
