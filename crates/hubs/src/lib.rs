//! The hubs this device knows: the status of each and sign-in.
//!
//! Owns: the registry of known hubs ([`Hubs`]) and the connection status every window shares
//! ([`HubStatus`]); the order of the sign-in steps ([`SignInSession`]); publishing a signed-in
//! server to `[hubs]`, and renaming, colouring, signing out of and removing a known one.
//!
//! Does not own: any I/O or protocol (the injected hub ports do, see `nocterm-hubs-model`);
//! the commands and dialog that show this state (`nocterm-hubs-ui`).
//!
//! Ports used: [`nocterm_hubs_model::HubTrust`], [`nocterm_hubs_model::HubSignIn`] and
//! [`nocterm_hubs_model::HubAdminKeys`] (the administrator keys waiting for confirmation, read
//! and confirmed through [`Hubs`]), passed in [`Deps`]; `nocterm_store::SettingsHandle`.
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
mod sign_in;
mod status;

pub use error::{HubsError, SignInError};
pub use installed::{Deps, Installed, install};
pub use registry::{HubSummary, Hubs, HubsEvent};
pub use sign_in::{SignInEvent, SignInSession, SignInStep, TrustNote};
pub use status::HubStatus;
