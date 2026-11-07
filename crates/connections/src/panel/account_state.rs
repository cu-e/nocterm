//! What the state of the account that keeps the list lets the user do, and the strip above
//! the list that says why something is unavailable.
//!
//! Without an account the list is only this computer's and always editable. With one, the
//! list is a copy: a change that is not delivered to the account is lost, so the list may be
//! changed only while the copy is in step with it.
use super::*;
use nocterm_connections_store::{ReplicationFailure, ReplicationStatus};
use nocterm_workspace::{UnlockVault, request_sign_in};

/// Where the copy of the account's list stands, read from the bound replication; none
/// without an account.
pub(crate) struct AccountState(Option<ReplicationStatus>);

impl AccountState {
    pub(crate) fn of(installed: &Installed, cx: &App) -> Self {
        Self(
            installed
                .replication()
                .map(|replication| replication.read(cx).status().clone()),
        )
    }

    /// Whether the list may be changed: always without an account, and with one only while
    /// the copy is in step with it, because a change that stays here is lost.
    pub(crate) fn editable(&self) -> bool {
        self.0.as_ref().is_none_or(editable)
    }

    /// Whether the list holds every connection the account keeps: always without an account,
    /// and with one only once the copy is in step. Until then a server saved on another device
    /// is not here yet.
    pub(crate) fn complete(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|status| matches!(status, ReplicationStatus::Synced))
    }

    /// What to tell the user about the account above the list.
    pub(crate) fn notice(&self) -> Option<AccountNotice> {
        self.0.as_ref().and_then(notice)
    }
}

/// Whether the list may be changed while the copy is in the state `status`.
pub(crate) fn editable(status: &ReplicationStatus) -> bool {
    matches!(
        status,
        ReplicationStatus::Synced | ReplicationStatus::Syncing
    )
}

/// What the user needs to hear about the account above the list.
#[derive(Clone, Debug)]
pub(crate) enum AccountNotice {
    /// The keys of the account are closed.
    Locked,
    /// The server cannot be reached; the list shown is the last one received.
    Offline,
    /// The account is signed out, so the list is empty.
    SignedOut,
    /// A connection was saved by a newer version of the application.
    NeedsNewerClient,
    Failed(ReplicationFailure),
}

/// The notice for a copy in the state `status`; none while it is in step.
pub(crate) fn notice(status: &ReplicationStatus) -> Option<AccountNotice> {
    match status {
        ReplicationStatus::Synced | ReplicationStatus::Syncing => None,
        ReplicationStatus::Detached => Some(AccountNotice::SignedOut),
        ReplicationStatus::Offline => Some(AccountNotice::Offline),
        ReplicationStatus::Locked => Some(AccountNotice::Locked),
        ReplicationStatus::NeedsNewerClient => Some(AccountNotice::NeedsNewerClient),
        ReplicationStatus::Failed(failure) => Some(AccountNotice::Failed(failure.clone())),
    }
}

impl AccountNotice {
    fn message(&self) -> String {
        match self {
            Self::Locked => "Account locked".to_owned(),
            Self::Offline => "Server unreachable — showing the last list, read-only".to_owned(),
            Self::SignedOut => "Signed out".to_owned(),
            Self::NeedsNewerClient => {
                "Some connections need a newer Nocterm — the list is read-only".to_owned()
            }
            Self::Failed(failure) => format!("Could not synchronize: {failure}"),
        }
    }

    /// The label of the button that resolves the notice, if the user can.
    fn remedy(&self) -> Option<&'static str> {
        match self {
            Self::Locked => Some("Unlock"),
            Self::SignedOut => Some("Sign in"),
            _ => None,
        }
    }

    fn resolve(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Locked => window.dispatch_action(Box::new(UnlockVault), cx),
            Self::SignedOut => {
                request_sign_in(window, cx);
            }
            _ => {}
        }
    }
}

/// The strip above the list for `notice`.
pub(super) fn strip(notice: &AccountNotice, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let remedy = notice.clone();
    h_flex()
        .id("connections-account-notice")
        .test_support()
        .gap_2()
        .mx_2()
        .mb_1()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(div().flex_1().min_w_0().child(notice.message()))
        .when_some(notice.remedy(), |strip, label| {
            strip.child(
                Button::new("connections-account-action")
                    .xsmall()
                    .label(label)
                    .on_click(move |_: &ClickEvent, window, cx| remedy.resolve(window, cx)),
            )
        })
}
