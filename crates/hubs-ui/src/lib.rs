//! The sign-in dialog and the Servers page of nocterm hubs.
//!
//! This crate gives the window the dialog that signs in to a server, the commands that sign in
//! to and out of one, and the Servers page of the settings.
//!
//! Owns: the presentation of the known hubs and their status; the Servers page of the settings
//! ([`settings_page`]), including the administrator keys waiting for the person's confirmation
//! (each with its fingerprint to compare out of band, and a warning when the key changed), and
//! the palette commands that sign in to and out of a server; the steps of
//! the sign-in dialog as the service orders them ([`register`]).
//!
//! Does not own: the registry of hubs and the order of sign-in (`nocterm-hubs`); hub I/O and
//! trust rules (`nocterm-hubs-model` and the injected adapter); terminal tabs.
//!
//! Ports: none of its own; it contributes no panel and no plugin.
//!
//! Settings: none of its own; the service owns the `[hubs]` section and the Servers page edits
//! it through the service.
//!
//! Globals: none.
//!
//! Invariants: a certificate that is not the trusted one is never offered for acceptance, on
//! the Servers page as in the dialog: removing the server is the way out; the trust step is never
//! confirmed by Enter alone; a password is cleared from its field when it is sent; closing the
//! sign-in dialog cancels a sign-in in flight.

mod color;
mod commands;
mod installation;
mod layout;
mod manage;
mod messages;
mod registration;
mod servers;
mod sign_in;
mod status;

pub use installation::{Installed, install};
pub use registration::register;
pub use servers::settings_page;

gpui_kit::actions!(
    hubs,
    [
        /// Sign in to a server that is not in the list yet.
        SignInToServer,
        /// Open the Servers page of the settings, where known servers are renamed, coloured, signed out of or removed.
        ManageServers,
        /// Choose a signed-in server and sign this device out of it.
        SignOutOfServer,
    ]
);

#[cfg(test)]
mod test_support;
