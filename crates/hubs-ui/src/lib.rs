//! The sign-in dialog, the Servers page and the sessions panel of nocterm hubs.
//!
//! The sessions panel lists the terminal sessions of one hub in the window the user is in, next
//! to the tabs of this computer, and starts new ones; it lets the user choose the hub when this
//! device knows several. This crate also gives the window the dialog that signs in to a server
//! and the Servers page of the settings.
//!
//! Owns: the presentation of the known hubs and their status; the Servers page of the settings
//! ([`settings_page`]) and the palette commands that sign in to and out of a server; the steps of
//! the sign-in dialog as the service orders them; the sessions panel and the choice of the hub it
//! lists ([`register`]).
//!
//! Does not own: the registry of hubs, sign-in order and session lists (`nocterm-hubs`); hub
//! I/O and trust rules (`nocterm-hubs-model` and the injected adapter); terminal tabs (the
//! application, through [`HubTerminalOpener`]).
//!
//! Ports: uses [`HubTerminalOpener`], implemented by the application; provides the
//! `hubs.sessions` panel of the `hubs` system plugin ([`plugin::PLUGIN`]).
//!
//! Settings: none of its own; the service owns the `[hubs]` section and the Servers page edits
//! it through the service.
//!
//! Globals: none.
//!
//! Invariants: a certificate that is not the trusted one is never offered for acceptance, on
//! the Servers page as in the panel: removing the server is the way out; the trust step is never
//! confirmed by Enter alone; a password is cleared from its field when it is sent; closing the
//! sign-in dialog cancels a sign-in in flight.

mod color;
mod commands;
mod installation;
mod layout;
mod manage;
mod messages;
mod ports;
mod problems;
mod registration;
mod servers;
mod sessions;
mod sign_in;
mod status;
mod tabs;

pub mod plugin;

pub use installation::{Installed, install};
pub use ports::{HubTabError, HubTerminalOpener};
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
        /// Show the sessions of a hub in the sidebar, or hide the sidebar if it shows them.
        OpenHubSessions,
    ]
);

#[cfg(test)]
mod test_support;
