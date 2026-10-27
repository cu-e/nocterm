//! The environment menu, the sign-in dialog and the sessions panel of nocterm hubs.
//!
//! A window shows one environment: this computer, or the terminal sessions of one hub. This
//! crate gives a window the title-bar menu that switches between them, the dialog that signs in
//! to a server, and, in a hub window, the panel that lists the sessions of the hub and the menu
//! that starts a new one.
//!
//! Owns: the presentation of the known hubs and their status ([`install_menu`]); the steps of
//! the sign-in dialog as the service orders them; the sessions panel and the new-session menu of
//! a hub window ([`register_hub_window`]); the confirmation before a window with tabs switches.
//!
//! Does not own: the registry of hubs, sign-in order and session lists (`nocterm-hubs`); hub
//! I/O and trust rules (`nocterm-hubs-model` and the injected adapter); composing windows
//! (the application, through [`EnvironmentOpener`]); terminal tabs (the application, through
//! [`HubTerminalOpener`]).
//!
//! Ports: uses [`EnvironmentOpener`] and [`HubTerminalOpener`], both implemented by the
//! application; provides the `hubs.sessions` panel ([`plugin::CORE`]).
//!
//! Settings: none of its own; the service owns the `[hubs]` section.
//!
//! Globals: none.
//!
//! Invariants: a certificate that is not the trusted one is never offered for acceptance;
//! the trust step is never confirmed by Enter alone; a password is cleared from its field when
//! it is sent; switching a window with tabs away from this computer asks first; closing the
//! sign-in dialog cancels a sign-in in flight.

mod color;
mod installation;
mod layout;
mod manage;
mod menu;
mod messages;
mod ports;
mod problems;
mod servers;
mod sessions;
mod sign_in;
mod status;
mod switch;
mod tabs;
mod window;

pub mod plugin;

pub use installation::{Installed, install};
pub use menu::install_menu;
pub use ports::{
    EnvironmentOpenError, EnvironmentOpener, HubTabError, HubTerminalOpener, Placement,
};
pub use servers::settings_page;
pub use window::{HubWindowError, register_hub_window};

gpui_kit::actions!(
    hubs,
    [
        /// Open the environment menu in the title bar.
        OpenEnvironmentMenu,
        /// Sign in to a server that is not in the list yet.
        SignInToServer,
        /// Show the sessions of the hub in the sidebar, or hide the sidebar if it shows them.
        OpenHubSessions,
    ]
);

#[cfg(test)]
mod test_support;
