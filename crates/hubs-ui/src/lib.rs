//! The environment menu, the sign-in dialog and the Servers page of nocterm hubs.
//!
//! A window shows one environment: this computer, or the account of one hub. This crate gives
//! the window the title-bar menu that switches between them, the dialog that signs in to a
//! server, the commands that sign in to, open and sign out of one, and the Servers page of the
//! settings.
//!
//! Owns: the presentation of the known hubs and their status ([`register`] installs the menu);
//! the Servers page of the settings ([`settings_page`]), a list of servers and, for the chosen
//! one, the tabs Overview, Devices (listed and revoked through the service), Keys (the vault
//! keys of the account, opened and closed on this device, and a new recovery key), Agents (the
//! account's agents and model providers, through `nocterm-hub-agents`) and Agent access (the
//! agent key and the servers connected to agents, checked and disconnected), and the palette
//! commands; the steps of the sign-in dialog as
//! the service orders them; the confirmation before a window with tabs switches.
//!
//! Does not own: the registry of hubs and the order of sign-in (`nocterm-hubs`); hub I/O and
//! trust rules (`nocterm-hubs-model` and the injected adapter); composing windows (the
//! application, through [`EnvironmentOpener`]); terminal tabs.
//!
//! Ports: uses [`EnvironmentOpener`], implemented by the application; it contributes no panel
//! and no plugin.
//!
//! Settings: none of its own; the service owns the `[hubs]` section and the Servers page edits
//! it through the service.
//!
//! Globals: none.
//!
//! Invariants: a certificate that is not the trusted one is never offered for acceptance, on
//! the Servers page as in the menu: removing the server is the way out; the trust step is never
//! confirmed by Enter alone; a password is cleared from its field when it is sent; switching a
//! window that has tabs asks first; closing the sign-in dialog cancels a sign-in in flight.

mod color;
mod commands;
mod installation;
mod layout;
mod manage;
mod menu;
mod messages;
mod ports;
mod problems;
mod servers;
mod sign_in;
mod status;
mod switch;

pub use installation::{Installed, install};
pub use menu::register;
pub use ports::{EnvironmentOpenError, EnvironmentOpener, Placement};
pub use servers::settings_page;
pub use sign_in::sign_in_again;

gpui_kit::actions!(
    hubs,
    [
        /// Open the environment menu in the title bar.
        OpenEnvironmentMenu,
        /// Sign in to a server that is not in the list yet.
        SignInToServer,
        /// Open the Servers page of the settings, where known servers are renamed, coloured, signed out of or removed.
        ManageServers,
        /// Choose a known server and open it in a new window.
        OpenServerInNewWindow,
        /// Choose a signed-in server and sign this device out of it.
        SignOutOfServer,
    ]
);

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod sign_in_action_tests {
    #[gpui_kit::test]
    fn the_sign_in_action_other_features_dispatch_by_name_exists(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            let action = cx
                .build_action(nocterm_workspace::SIGN_IN_TO_SERVER_ACTION, None)
                .expect("registered");
            assert!(action.partial_eq(&crate::SignInToServer));
        });
    }
}
