//! Hub adapter: reaches Nocterm hubs through the external hub contract.
//!
//! This is the only crate that knows the hub protocol exists. It speaks it through
//! `nocterm-hub-client` (pinned TLS, signed requests, token refresh) and shows the rest of the
//! application the neutral ports of `nocterm-hubs-model`, `nocterm-hub-agents-model`,
//! `nocterm-ai`, `nocterm-vault`, `nocterm-connections-model` and `nocterm-session`. Like
//! `nocterm-ssh`, it owns a small Tokio runtime, so callers need none: every call returns a
//! runtime-agnostic future.
//!
//! Owns: connections to hubs (one client per hub), the probe of a hub's certificate, password
//! and browser sign-in and sign-out (which revokes the device on the hub), the private
//! credential file of this device for each hub with its inter-process lock, the mapping of hub
//! failures to `HubError`, the mapping of hub records to the model's records, the relayed byte
//! streams of SSH routes that use a hub as their first step, and the encrypted data of one
//! signed-in account: the account key (opened at sign-in, unlock and recovery, zeroed on lock),
//! the vault collections sealed under it, their encrypted local cache, the vault ([`HubVault`])
//! and the profile replica ([`HubProfileReplica`]) built on them, handed out together as a
//! [`HubAccount`].
//! Agents, providers and servers (`agents`, `servers`) implement the ports of
//! `nocterm-hub-agents-model` over `AgentService` and `ServerService`; [`HubAgentConnector`]
//! implements `nocterm_ai::AgentConnector` over `ChatService` (a hub agent's chat is an
//! ordinary agent chat whose session is the hub chat), and [`HubChatRepository`] lists, renames,
//! pins and deletes hub chats beside the local history of a hub window.
//! Does not own: which hubs the user knows or trusts (the `[hubs]` settings section and the
//! trust decision belong to `nocterm-hubs-model` and the callers), connection status, retries,
//! windows, terminal emulation, the transcript of a chat, or any view.
//! Ports provided: `HubTrust`, `HubSignIn` and `AccountDevices` from `nocterm-hubs-model`
//! (by [`HubLinks`]) and `AccountKeyLock` (by [`AccountKeys`]); `HubAgentDirectory`,
//! `HubServers` and `HubChatChanges` (by [`HubAgentsLink`]); `AgentConnector` and
//! `ChatRepository` of `nocterm-ai`; the `LocalAccountSource` captured by [`HubAccount`], its
//! captured `CredentialStore` of `nocterm-session`, the `VaultAccess` of `nocterm-vault` and
//! the `ProfileReplica` of `nocterm-connections-model`, implemented by [`HubVault`] and
//! [`HubProfileReplica`], and the `RelayDialer` of `nocterm-session`, implemented by
//! [`HubRelay`] over the clients of the hubs this device is signed in to. Ports used: none.
//! Settings section: none; the callers pass the directory for credentials.
//! Globals: none.
//! Invariants: the device key and tokens live only in a mode-0600 file of a mode-0700
//! directory, published atomically and read under an inter-process lock; a pinned hub is
//! trusted only with the exact pinned certificate; a revoked device erases its credentials, its
//! account cache and its keys; the account key lives only in memory, is zeroed on lock and
//! follows its own lock policy, independent of the local vault's; only ciphertext and non-secret
//! facts are written to disk (mode 0600, atomically); the local history partition of a hub
//! window depends on the signed-in account only, so a locked vault never blocks chats; no tonic
//! or prost type crosses the crate boundary, and no text built from a failure does; a relay
//! finds its hub only among the hubs signed in to and never signs in; every buffer between the
//! transport and the hub's stream is bounded, and dropping either end of a relayed stream ends
//! both directions; a record of the hub that does not authenticate is an error, never a silent
//! skip or overwrite; collections and record kinds this version does not know are never
//! listed, changed or deleted.

mod account;
mod account_keys;
mod agents;
mod cache;
mod chats;
mod checked;
mod classify;
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
mod store;
mod task;
mod vault;

pub use account_keys::AccountKeys;
pub use agents::HubAgentsLink;
pub use chats::{HubAgentConnector, HubChatRepository};
pub use hub_account::HubAccount;
pub use links::{HubLinks, HubLinksConfig, HubStartError};
pub use relay::HubRelay;
pub use replica::HubProfileReplica;
pub use vault::HubVault;
