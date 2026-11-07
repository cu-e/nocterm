//! The hubs this device knows and the neutral ports through which the client reaches a hub.
//!
//! Owns: `Environment` (what a window shows: this computer or one hub), hub addresses and ids,
//! certificate fingerprints and the trust decision, captured descriptive hub/account source
//! identities (`source_identity`, distinct from address and certificate observations),
//! the colour that marks a server
//! (`HubColor`), the `[hubs]` settings section (`HubsSettings`), the hub access
//! ports `HubTrust`, `HubSignIn` with their records and `HubError`,
//! including the status of the account keys after sign-in (`KeyStatus`), opaque
//! recovery issue/acknowledgement capabilities (`RecoveryKeyReissue`, `RecoveryKeyOffer`),
//! and `LoginReceipt` for original-login cleanup after delayed settings publication.
//! Does not own: the hub wire protocol and its SDK types (the hub contract crates, reached
//! only by `nocterm-hub-io`), device keys and tokens (private adapter files), connection
//! state over time (`nocterm-hubs`), views.
//! Ports provided: HubTrust, HubSignIn, AccountKeyLock (the account keys and their lock policy)
//! and AccountDevices (the devices of an account, listed and revoked). Ports used:
//! nocterm-session `Secret`.
//! Settings section: `hubs`. Globals: none.
//! Invariants: no I/O; no protocol types; a hub is identified by its canonical address;
//! captured source identities include hub authority and account-key fingerprints, never display
//! addresses or local project ids; their constructors do not authenticate directory values;
//! displayed confirmations carry injected original-recipient capabilities, never authority
//! reconstructed from display ids; stale recipient/action errors stay typed;
//! a pinned fingerprint never changes silently: a different certificate is an error, not a
//! prompt; plaintext addresses are refused; secrets never appear in `HubsSettings`.
//!
//! With the `test-support` feature, [`testing::FakeHub`] implements the two access ports in
//! memory for consumers' tests.

//! LocalAccountSource/LocalAccountSourceFactory are consumer ports for captured native
//! account owners. Identity constructors validate shape; adapters authenticate values, and
//! lease checks prevent old owners from returning after sign-out or key replacement.

pub mod account_keys;
pub mod address;
pub mod color;
pub mod devices;
pub mod environment;
pub mod error;
mod login;
pub use login::{LoginOwner, LoginReceipt};
pub mod ports;
pub mod records;
mod recovery;
pub use recovery::{
    RecoveryKeyConfirmation, RecoveryKeyIssuer, RecoveryKeyOffer, RecoveryKeyReissue,
};
pub mod settings;
pub mod source_identity;
pub mod trust;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use account_keys::{AccountKeyError, AccountKeyFuture, AccountKeyLock};
pub use address::{DEFAULT_PORT, HubAddress, HubId};
pub use color::HubColor;
pub use devices::{AccountDevices, DeviceSummary};
pub use environment::Environment;
pub use error::{FingerprintError, HubAddressError, HubError};
pub use ports::{HubFuture, HubSignIn, HubTrust, OidcSignIn};
pub use records::{
    Account, AccountId, HubEndpointSpec, HubInfo, KeyStatus, OidcProvider, SignedIn,
};
pub use settings::{HubRecord, HubsSettings};
pub use trust::{Fingerprint, ProbedHub, TrustDecision, TrustPolicy, decide};
