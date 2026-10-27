//! The environment a window belongs to, the hubs this device knows and the neutral ports
//! through which the client reaches a hub.
//!
//! Owns: `Environment` (local or one hub), the document of open windows and its rules
//! (`WindowDocument`, `OpenWindows`), hub addresses and ids, certificate fingerprints
//! and the trust decision, the `[hubs]` settings section (`HubsSettings`), the hub access
//! ports `HubTrust`, `HubSignIn`, `HubSessions` with their records and `HubError`.
//! Does not own: the hub wire protocol and its SDK types (the hub contract crates, reached
//! only by `nocterm-hub-io`), device keys and tokens (private adapter files), connection
//! state over time (`nocterm-hubs`), views.
//! Ports provided: HubTrust, HubSignIn, HubSessions. Ports used: nocterm-session `Secret`.
//! Settings section: `hubs`. Globals: none.
//! Invariants: no I/O; no protocol types; the window document is versioned and its records
//! ignore fields they do not know, so a record can later carry per-window hub data without a
//! format break, and a record that cannot be read costs only itself; the last open window is never forgotten; a hub is identified by its canonical address;
//! a pinned fingerprint never changes silently: a different certificate is an error, not a
//! prompt; plaintext addresses are refused; secrets never appear in `HubsSettings`.
//!
//! With the `test-support` feature, [`testing::FakeHub`] implements all three ports in
//! memory for consumers' tests.

pub mod address;
pub mod environment;
pub mod error;
pub mod ports;
pub mod records;
pub mod settings;
pub mod trust;
pub mod windows;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use address::{DEFAULT_PORT, HubAddress, HubId};
pub use environment::Environment;
pub use error::{FingerprintError, HubAddressError, HubError, WindowDocumentError};
pub use ports::{HubFuture, HubSessions, HubSignIn, HubTrust, OidcSignIn};
pub use records::{
    Account, AccountId, HubEndpointSpec, HubInfo, HubSessionId, HubSessionState, HubSessionSummary,
    HubTerminalTarget, NodeId, NodeSummary, OidcProvider, OrgId, OrgSummary,
};
pub use settings::{HubRecord, HubsSettings};
pub use trust::{Fingerprint, ProbedHub, TrustDecision, TrustPolicy, decide};
pub use windows::{OpenWindows, WindowDocument};
