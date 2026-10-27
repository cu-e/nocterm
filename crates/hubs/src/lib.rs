//! The hubs this device knows: the status of each, sign-in, and the lists of its sessions.
//!
//! Owns: the registry of known hubs ([`Hubs`]) and the connection status every window shares
//! ([`HubStatus`]); the order of the sign-in steps ([`SignInSession`]); the account, nodes and
//! sessions the sessions panel shows ([`HubSessionLists`]); publishing a signed-in server to `[hubs]`, and
//! renaming, colouring, signing out of and removing a known one.
//!
//! Does not own: any I/O or protocol (the injected hub ports do, see `nocterm-hubs-model`);
//! the commands, dialog and panel that show this state (`nocterm-hubs-ui`).
//!
//! Ports used: [`nocterm_hubs_model::HubTrust`], [`nocterm_hubs_model::HubSignIn`] and
//! [`nocterm_hubs_model::HubSessions`], passed in [`Deps`]; `nocterm_store::SettingsHandle`.
//! Provides: [`Installed`] with the [`Hubs`] entity.
//!
//! Settings: the `[hubs]` section ([`nocterm_hubs_model::HubsSettings`]), registered by
//! [`install`].
//!
//! Globals: none.
//!
//! Invariants: a certificate that is not the trusted one is never retried and never
//! accepted; an unreachable hub is retried with growing delays; a server enters the settings
//! only after a sign-in worked, in one edit, and credentials made for a server that could not
//! be saved are erased again; every window shares one status and one connection per hub.

mod error;
#[cfg(test)]
mod fixture;
mod installed;
mod registry;
mod session_lists;
mod sign_in;
mod status;

pub use error::{HubsError, SignInError};
pub use installed::{Deps, Installed, install};
pub use registry::{HubSummary, Hubs, HubsEvent};
pub use session_lists::{HubSessionLists, HubSessionListsEvent, Load, Watch};
pub use sign_in::{SignInEvent, SignInSession, SignInStep, TrustNote};
pub use status::HubStatus;
