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
//! Native cache writers stage encoding, writes and file sync on two shared
//! blocking slots. Final publication rechecks original authority and immutable
//! content generation under admission; parent sync follows outside that lock.
//! Confirmation futures acknowledge durable publication before returning.
//! Personal refreshes coalesce waiting readers; four shared vault operations
//! bound staging, and sealed snapshots reject excess bytes, rows and pages.
//! The personal SSH CA for cloud agents lives in `agent_access`: its seed and the registry of
//! trusting servers are objects of the account's own `agent_ssh_trust` collection, and the
//! `AgentHostTrust` and `AgentGrants` ports are implemented there ([ADR 0119](../../../docs/adr/0119-agent-ssh-ca.md)).
//! Native Cloud adapter owners also keep the runner's three purpose-specific keys,
//! original enrolled source and durable rotated session, authenticated chat material,
//! signed public coordinator observations and exact ciphertext publication metadata.
//! Runtime authority retains accepted hard deadlines across known credential rotation;
//! RPC readiness is separate, and unknown refresh/expiry permanently fences it.
//! Publication and argument encoding use the public canonical codec with injected
//! finite work reservations; native journal framing never supplies a wire payload.
//! Provider boot announcements/status use guarded SDK RPCs and independently
//! installed runner signing pins; hub receipt times never establish freshness.
//! Account/R catalog pages use the original normal unlocked account and trusted
//! runner registration before any ChatKey; opaque resident pages bind fresh
//! signed revisions/cursors and preserve deleted and zero-event heads.
//! Account Cloud material uses only a dedicated personal encrypted collection.
//! Normal account keys also open an immutable hub-authority bootstrap record;
//! explicit setup verifies the candidate capability before creating trust.
//! Capability pinning and actual native membership precede stable scope checks;
//! normal unlocked account keysets restore wrapped chat keys on another device.
//! Material facades keep weak key ownership, immutable runner pins and actual
//! allocation leases; they never retain a plaintext chat key across operations.
//! Captured Cloud conversations provide the neutral `CapturedConversationSource`
//! port: nonce-authenticated current heads, bounded signed event replay, durable
//! exact-command outbox and one encrypted verified-page cache. Installation-owned
//! finite workers/storage drivers survive a view or original membership ending;
//! each publication still checks that original membership. Sparse snapshot
//! activation and first-chat creation are separate consumer transitions.
//! Does not own: which hubs the user knows or trusts (the `[hubs]` settings section and the
//! trust decision belong to `nocterm-hubs-model` and the callers), connection status, retries,
//! windows, terminal emulation, or any view.
//! Ports provided: `HubTrust`, `HubSignIn`, `HubAdminKeys` and `AccountDevices` from
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
//!
//! The managed-agent ports of `nocterm-cloud-agents-model` (`agents`) read the failure code,
//! the web panel address and the provider kinds through the contract's own vocabulary
//! (`ProvisionFailure`, `web_url`, `ProviderKind`); a seat is stopped or removed through the
//! caller's own instances of the agent.
//!
//! Cloud chats (`cloud_chats`) implement the `CloudChatLauncher` port of
//! `nocterm-cloud-agents-model`: the device mints a chat and its first key epoch, stores the
//! wrapped key and the sealed runner grant in the account before publishing, waits (bounded,
//! republishing) for the runner to claim the chat, and lists chats from the authenticated
//! catalogs of the registered runners. Managed chats start their seat for the chat first and
//! prompt at the runner's default destination
//! ([ADR 0122](../../../docs/adr/0122-cloud-chat-creation-protocol.md)); their source wakes
//! the seat again before it delivers a command the runner must admit
//! ([ADR 0133](../../../docs/adr/0133-cloud-chats-in-the-history-column-and-seats-woken-for-prompts.md)).

mod account;
mod account_keys;
pub mod agent_access;
mod agents;
mod cache;
mod checked;
mod classify;
pub mod cloud;
mod cloud_chats;
mod connect;
mod convert;
mod credentials;
mod devices;
mod directory;
mod endpoint;
mod hub_account;
mod links;
mod pool;
mod relay;
mod replica;
pub mod runner_control;
mod store;
mod task;
mod vault;

pub use account::{AdminKeyError, AdminKeys, PendingAdminKey};
pub use account_keys::AccountKeys;
pub use agents::{CloudAgentsLink, parse_provider_kind};
pub use cloud_chats::{ChatServices, CloudChatsLink};
pub use hub_account::HubAccount;
pub use links::{HubLinks, HubLinksConfig, HubStartError};
pub use relay::HubRelay;
pub use replica::HubProfileReplica;
pub use vault::HubVault;
