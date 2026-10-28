//! The hubs this device knows and the neutral ports through which the client reaches a hub.
//!
//! Owns: hub addresses and ids, certificate fingerprints and the trust decision, the colour
//! that marks a server (`HubColor`), the `[hubs]` settings section (`HubsSettings`), the hub
//! access ports `HubTrust`, `HubSignIn`, `HubSessions` with their records and `HubError`,
//! including the status of the account keys after sign-in (`KeyStatus`) and the choice of the
//! account server (`HubsSettings::account_hub`), and the administrator keys waiting for a
//! person's confirmation (`HubAdminKeys`, `PendingAdminKey`).
//! Does not own: the hub wire protocol and its SDK types (the hub contract crates, reached
//! only by `nocterm-hub-io`), device keys and tokens (private adapter files), connection
//! state over time (`nocterm-hubs`), views.
//! Ports provided: HubTrust, HubSignIn, HubSessions, HubAdminKeys. Ports used: nocterm-session
//! `Secret`.
//! Settings section: `hubs`. Globals: none.
//! Invariants: no I/O; no protocol types; a hub is identified by its canonical address;
//! a pinned fingerprint never changes silently: a different certificate is an error, not a
//! prompt; plaintext addresses are refused; secrets never appear in `HubsSettings`.
//!
//! With the `test-support` feature, [`testing::FakeHub`] implements the three access ports and
//! [`testing::FakeAdminKeys`] the administrator keys, in memory, for consumers' tests.

pub mod address;
pub mod admin_keys;
pub mod color;
pub mod error;
pub mod ports;
pub mod records;
pub mod settings;
pub mod trust;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use address::{DEFAULT_PORT, HubAddress, HubId};
pub use admin_keys::{AdminKeyError, HubAdminKeys, PendingAdminKey};
pub use color::HubColor;
pub use error::{FingerprintError, HubAddressError, HubError};
pub use ports::{HubFuture, HubSessions, HubSignIn, HubTerminal, HubTrust, OidcSignIn};
pub use records::{
    Account, AccountId, HubEndpointSpec, HubInfo, HubSessionId, HubSessionState, HubSessionSummary,
    HubTerminalTarget, KeyStatus, NodeId, NodeSummary, OidcProvider, OrgId, OrgSummary, SignedIn,
};
pub use settings::{HubRecord, HubsSettings};
pub use trust::{Fingerprint, ProbedHub, TrustDecision, TrustPolicy, decide};
