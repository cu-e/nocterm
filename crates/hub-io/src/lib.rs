//! Hub adapter: reaches Nocterm hubs through the external hub contract.
//!
//! This is the only crate that knows the hub protocol exists. It speaks it through
//! `nocterm-hub-client` (pinned TLS, signed requests, token refresh) and shows the rest of the
//! application the neutral ports of `nocterm-hubs-model`. Like `nocterm-ssh`, it owns a small
//! Tokio runtime, so callers need none: every call returns a runtime-agnostic future.
//!
//! Owns: connections to hubs (one client per hub), the probe of a hub's certificate, password
//! and browser sign-in, the private credential file of this device for each hub with its
//! inter-process lock, the mapping of hub failures to `HubError`, the mapping of hub records to
//! the model's records, and the relayed byte streams of SSH routes that use a hub as
//! their first step (the pump between the transport's bounded frame channels and the hub's
//! stream, and the mapping of the hub's refusals to `RelayError`), and the encrypted data of
//! one signed-in account: the account keys (opened at sign-in and recovery, zeroed on lock), the
//! store of sealed collections, the encrypted cache of the vault records, the vault
//! ([`HubVault`]) and the profile replica ([`HubProfileReplica`]) built on them, which
//! [`HubLinks::open_account`] hands out together as a [`HubAccount`]. It also captures
//! device-local history identity through `LocalAccountSource`: configured hub,
//! actual account and the open account key's fingerprint, without claiming a stable
//! Cloud authority for older hubs. `HubAccount::local_credentials` binds terminal workers
//! to the original keys, pooled membership and client, with cache publication conditional
//! under the account Slot mutex. Its facade retains no open key material. Next to the personal
//! collection the account reads the groups of its organizations: collections of a kind whose
//! key was granted to the member, listed with the member's role. A group is left out when its
//! key is not granted or does not open; the client never creates, deletes or grants one
//! (`hubctl` does). A Guest only reads; the replica reports a refused write as `Forbidden`.
//! Administrators are trusted on first use, with a fingerprint: a group's key opens only under
//! the key of its granter pinned on this device after a person compared fingerprints, so a
//! group whose granter is not pinned, or whose key changed, is left out and listed as pending
//! ([`AdminKeys`], reached through the `HubAdminKeys` port); confirming raises the keys notice.
//! Native account workers capture their membership and store before enqueue; replica facades
//! remain bound to their original opened source. The general vault captures authenticated
//! membership separately from open keys, keeping explicit unlock available while locked.
//! Slot lifecycle receipts fence key installation, and native admission makes cache, spaces
//! and idle-timer publication conditional. Administrator and recovery offers retain the
//! original recipient and exact displayed action ([ADR 0114](../../../docs/adr/0114-native-captured-account-operations.md)).
//! Native Cloud adapter owners also keep the runner's three purpose-specific keys,
//! original enrolled source and durable rotated session, authenticated chat material,
//! signed public coordinator observations and exact ciphertext publication metadata.
//! Publication and argument encoding use the public canonical codec with injected
//! finite work reservations; native journal framing never supplies a wire payload.
//! Does not own: which hubs the user knows or trusts (the `[hubs]` settings section and the
//! trust decision belong to `nocterm-hubs-model` and the callers), connection status, retries,
//! windows, terminal emulation, or any view.
//! Ports provided: `HubTrust`, `HubSignIn` and `HubAdminKeys` from
//! `nocterm-hubs-model`, all implemented by [`HubLinks`], `LocalAccountSource` captured
//! by [`HubAccount`], its captured `CredentialStore` of `nocterm-session`, the `VaultAccess` of `nocterm-vault` and the `ProfileReplica` of
//! `nocterm-connections-model`, implemented by [`HubVault`] and [`HubProfileReplica`], and the `RelayDialer` of `nocterm-session`, implemented
//! by [`HubRelay`] over the clients of the hubs this device is signed in to. Ports used: none.
//! Settings section: none; the callers pass the directory for credentials.
//! Globals: none.
//! Invariants: the device key and tokens live only in a mode-0600 file of a mode-0700
//! directory, published atomically and read under an inter-process lock; a pinned hub is
//! trusted only with the exact pinned certificate; a revoked device erases its credentials, its
//! account cache and its keys; a recovery key shown and not confirmed is flagged in the cache (a
//! fact, not a secret) and replaced by a new one through a keyset write conditional on its
//! version, never by overwriting a record whose key pair is not the open one;
//! no tonic or prost type crosses the crate boundary, and no text built from a failure does;
//! a relay finds its hub only among the hubs signed in to and never signs in; every
//! buffer between the transport and the hub's stream is bounded, and dropping either end of a
//! relayed stream ends both directions, whether or not the target is silent; account keys and
//! the unlock secret live only in memory and are zeroed on lock, and only ciphertext, non-secret
//! facts and the public keys of pinned administrators are written to disk (mode 0600, atomically);
//! a pin is never added or replaced without a person's confirmation, and is erased with the
//! account; a local history capture holds only weak key and login membership
//! references, so lock, eviction or replacement login cannot revive it; ordinary
//! refresh preserves that membership and never becomes a Cloud authority;
//! each SDK token store adopts and saves only its original account/device; a late
//! captured operation never selects or erases a replacement account;
//! a record of the hub that does not authenticate is an error,
//! never a silent skip or overwrite; collections and record kinds this version does not know are
//! never listed, changed or deleted; each collection has its own key.

//! Captured store and relay consumers share original SDK client/membership
//! selection. Relay capture happens before worker spawn, binds the exact route,
//! and rechecks source after awaits. Late failures retire only original ownership.
//!
//! Cloud native ownership lives in [`cloud`]: explicit persistent runner identity
//! and purpose-specific journal catalog/head cryptography through cloud-model ports.
//! Its encryption, transcript-signing and Cloud-token proof keys are independent;
//! setup/open never resets an existing identity. It exports public identity facts
//! and neutral cipher ports, never a generic signer, private seed or SDK wire type.

mod account;
mod cache;
mod classify;
pub mod cloud;
mod connect;
mod convert;
mod credentials;
mod directory;
mod endpoint;
mod hub_account;
mod links;
mod pool;
mod relay;
mod replica;
mod store;
mod task;
mod vault;

pub use account::{AdminKeyError, AdminKeys, PendingAdminKey};
pub use hub_account::HubAccount;
pub use links::{HubLinks, HubLinksConfig, HubStartError};
pub use relay::HubRelay;
pub use replica::HubProfileReplica;
pub use vault::HubVault;
