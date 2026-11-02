//! What the state of the hub account behind this window's chats lets the person do, and the
//! strip that says why the chats are unavailable.
use gpui_kit::{
    App, ClickEvent, IntoElement, TestSupportExt as _, Window,
    component::{ActiveTheme as _, Sizable as _, button::Button, h_flex},
    div,
    prelude::*,
};
use nocterm_ai::storage::AgentStorageError;
use nocterm_hubs_model::HubError;
use nocterm_workspace::UnlockVault;

use nocterm_agent_runtime::CatalogLoadError;

/// The action that signs in to a server; named, because its crate depends on this one.
const SIGN_IN_ACTION: &str = "hubs::SignInToServer";

/// Why the account of this window cannot serve chats right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AccountNotice {
    /// The keys of the account are closed.
    Locked,
    /// The account is signed out on this device.
    SignedOut,
    /// The account the window captured is gone or cannot be identified.
    Unavailable,
}

impl AccountNotice {
    /// The notice for a failure to capture the account, if the failure is about the account.
    pub(super) fn from_capture(error: &AgentStorageError) -> Option<Self> {
        match error {
            AgentStorageError::AccountCapture(HubError::KeysLocked) => Some(Self::Locked),
            AgentStorageError::AccountCapture(HubError::NotSignedIn | HubError::Revoked) => {
                Some(Self::SignedOut)
            }
            AgentStorageError::AccountCapture(_) | AgentStorageError::AccountSource(_) => {
                Some(Self::Unavailable)
            }
            _ => None,
        }
    }

    /// The notice for a failed history load, if the failure is about the account.
    pub(super) fn from_catalog(error: &CatalogLoadError) -> Option<Self> {
        match error {
            CatalogLoadError::AccountSource(_) => Some(Self::Unavailable),
            CatalogLoadError::Storage(error) => Self::from_capture(error),
            _ => None,
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Locked => "Hub account locked — unlock it to see its chats",
            Self::SignedOut => "Signed out of the hub — sign in to see its chats",
            Self::Unavailable => "Hub account unavailable — unlock it or sign in again",
        }
    }

    fn remedy(self) -> &'static str {
        match self {
            Self::SignedOut => "Sign in",
            Self::Locked | Self::Unavailable => "Unlock",
        }
    }

    fn resolve(self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Locked | Self::Unavailable => {
                window.dispatch_action(Box::new(UnlockVault), cx);
            }
            Self::SignedOut => {
                if let Ok(action) = cx.build_action(SIGN_IN_ACTION, None) {
                    window.dispatch_action(action, cx);
                }
            }
        }
    }

    /// The strip shown above the chats.
    pub(super) fn strip(self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .id("agent-account-notice")
            .test_support()
            .gap_2()
            .mx_2()
            .my_1()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .bg(theme.muted)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().flex_1().min_w_0().child(self.message()))
            .child(
                Button::new("agent-account-action")
                    .xsmall()
                    .label(self.remedy())
                    .on_click(move |_: &ClickEvent, window, cx| self.resolve(window, cx)),
            )
    }
}
