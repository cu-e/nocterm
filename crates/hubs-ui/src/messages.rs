//! The words the views use for the failures of the hub service. The service reports typed
//! errors; this is the only place that turns them into text.

use nocterm_hubs::{HubsError, SignInError};
use nocterm_hubs_model::{AdminKeyError, HubError};

/// Why a sign-in step did not work, for the dialog.
pub(crate) fn sign_in_problem(error: &SignInError) -> String {
    match error {
        SignInError::Address(error) => capitalize(&error.to_string()),
        SignInError::Fingerprint(error) => capitalize(&error.to_string()),
        SignInError::Hub(error) => hub_problem(error),
        SignInError::AlreadyKnown(hub) => format!(
            "{hub} is in the list already. Use its row to sign in again, or remove it first."
        ),
        SignInError::NotInitialized => {
            "This server has no owner yet. Set it up first, then sign in here.".to_owned()
        }
        SignInError::Save(_) => {
            "The server could not be saved to the settings, so this device was signed out of it again."
                .to_owned()
        }
    }
}

/// Why forgetting a server or signing out of it did not work.
pub(crate) fn hubs_problem(error: &HubsError) -> String {
    match error {
        HubsError::Unknown(hub) => format!("{hub} is not in the list."),
        HubsError::Hub(error) => hub_problem(error),
        HubsError::Save(_) => {
            "The settings could not be saved, so the server stays listed.".to_owned()
        }
    }
}

/// Why an administrator's key was not confirmed.
pub(crate) fn admin_key_problem(error: AdminKeyError) -> String {
    match error {
        AdminKeyError::Stale => "The account changed. Compare the key shown for the current account again.".to_owned(),
        AdminKeyError::Locked => {
            "The account keys are closed on this device. Unlock the vault first.".to_owned()
        }
        AdminKeyError::NotPending => {
            "That administrator's key is no longer waiting for confirmation.".to_owned()
        }
        AdminKeyError::Mismatch => {
            "The server now offers another key than the one shown. Compare the new fingerprint with the administrator's before confirming."
                .to_owned()
        }
    }
}

/// What a failed call on a hub means to the person using it.
pub(crate) fn hub_problem(error: &HubError) -> String {
    match error {
        HubError::Unreachable { address, .. } => {
            format!("Cannot reach {address}. Check the address and your network.")
        }
        HubError::CertificateChanged { address, .. } => {
            format!("The certificate of {address} is not the one that was pinned.")
        }
        HubError::CertificateUntrusted { address, .. } => {
            format!("The certificate of {address} is not trusted.")
        }
        HubError::UpgradeRequired { hub, client } => format!(
            "This server speaks protocol {hub} and Nocterm speaks {client}. Update the older one."
        ),
        HubError::InvalidCredentials => "The user name or password was not accepted.".to_owned(),
        HubError::Stale => "The account changed. Start this action again for the current account.".to_owned(),
        HubError::RecoveryKeyInvalid => {
            "That is not the recovery key of this account. Check it and try again.".to_owned()
        }
        HubError::InvalidVaultPassphrase => {
            "The vault passphrase is too short. Use at least eight characters.".to_owned()
        }
        HubError::KeysLocked => {
            "The account keys are closed on this device. Unlock the vault or sign in with your password first."
                .to_owned()
        }
        HubError::UnlockSourceMismatch {
            needs_login_password: true,
        } => "These account keys are opened with your login password, not with a vault passphrase. Sign in with the password instead."
            .to_owned(),
        HubError::UnlockSourceMismatch {
            needs_login_password: false,
        } => "These account keys are opened with a vault passphrase, not with your login password. Type the passphrase."
            .to_owned(),
        HubError::KeysetDamaged { .. } => {
            "The account keys on the server do not verify. The server may have been tampered with; nothing was changed."
                .to_owned()
        }
        other => capitalize(&other.to_string()),
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>() + ".",
        None => String::new(),
    }
}

/// Why an agent access change did not work.
pub(crate) fn agent_problem(error: &nocterm_hubs_model::AgentAccessError) -> String {
    use nocterm_hubs_model::AgentAccessError as Error;
    match error {
        Error::Hub(error) => hub_problem(error),
        Error::NotConnected { connection } => format!(
            "No session to {connection} is open. Connect to the server first, then try again."
        ),
        Error::HostChanged => {
            "The server's authorized_keys changed while it was edited. Nothing was written; try again."
                .to_owned()
        }
        Error::HostUnsupported => {
            "The server's authorized_keys cannot be edited here. Add or remove the line by hand."
                .to_owned()
        }
        Error::Host { .. } => "The server could not be edited.".to_owned(),
        Error::Unknown { connection } => format!("No server is registered for {connection}."),
        Error::Records { .. } => {
            "The agent access records on this device are unusable.".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_hubs_model::HubAddressError;

    #[test]
    fn a_refused_address_reads_as_a_sentence() {
        let error = SignInError::Address(HubAddressError::Empty);
        assert_eq!(sign_in_problem(&error), "The hub address is empty.");
    }

    #[test]
    fn refused_credentials_name_the_user_name_and_password() {
        assert_eq!(
            hub_problem(&HubError::InvalidCredentials),
            "The user name or password was not accepted."
        );
    }
}
