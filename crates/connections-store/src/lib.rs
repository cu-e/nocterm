//! Ordered connection publication and bounded raw server-fact acquisition.
//! Owns: checked profiles/recents, committed publication carrier, raw Known/flag
//! state, original-source tickets, policy history, physical reservations and capture.
//! Does not own: native I/O, views/images, session providers, settings storage,
//! credentials, terminal implementation or observation composition.
//! Ports used: supplied writers/clock, DNS/country/flag/OS observation contracts,
//! CountryPolicySource, ProbeSourceAuthority, BackgroundExecutor, ShutdownCoordinator.
//! Ports provided: ConnectionStore, ProfilePublications, FactsService.
//! Settings: supplied enabled/revision policy projection. Globals: none.
//! Invariants: committed carrier and current original source/policy are read at
//! admission/completion; stale jobs keep their physical slot until actual finish;
//! 32 slots are shared across windows and paired reservations are atomic. Facts
//! use common optimistic persistence/capture and its existing 180-ms deadline.
mod connections;
mod error;
mod prepared;
mod publication;
pub use connections::ConnectionStore;
pub use error::ConnectionsError;
pub use nocterm_connections_model::profile_authority::{
    ProfileAuthorityRow, ProfilePublication, ProfileStamp,
};
pub use publication::projection::ProfilePublications;

mod facts;
mod observation;
pub use facts::{
    CountryPolicySnapshot, CountryPolicySource, FactWorkload, FactsAdmissionError, FactsEvent,
    FactsPrepared, FactsService, FlagState, MAX_FACT_JOBS, ProbeOutcome, ProbeRequest,
    ProbeSourceAuthority, ProbeSourceIdentity, ProbeSourceLease,
};
pub use observation::ObservationDeps;
