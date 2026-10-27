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
//! the model's records, and the terminal transport of hub sessions (the contract's terminal
//! events as session events).
//! Does not own: which hubs the user knows or trusts (the `[hubs]` settings section and the
//! trust decision belong to `nocterm-hubs-model` and the callers), connection status, retries,
//! windows, terminal emulation, or any view.
//! Ports provided: `HubTrust`, `HubSignIn` and `HubSessions` from `nocterm-hubs-model`, all
//! implemented by [`HubLinks`], and the `Transport` of `nocterm-session` that `HubSessions::terminal`
//! returns for one attached hub session. Ports used: none.
//! Settings section: none; the callers pass the directory for credentials.
//! Globals: none.
//! Invariants: the device key and tokens live only in a mode-0600 file of a mode-0700
//! directory, published atomically and read under an inter-process lock; a pinned hub is
//! trusted only with the exact pinned certificate; a revoked device erases its credentials;
//! no tonic or prost type crosses the crate boundary, and no text built from a failure does
//! except the reason a terminal session ended, in the text variants `SessionError` already has;
//! closing a terminal transport's session detaches from it and never ends the process on its
//! node.

mod classify;
mod connect;
mod convert;
mod credentials;
mod directory;
mod endpoint;
mod links;
mod pool;
mod terminal;

pub use links::{HubLinks, HubLinksConfig, HubStartError};
