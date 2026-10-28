//! The words the views use for the failures of the hub service. The service reports typed
//! errors; this is the only place that turns them into text.

use nocterm_hubs::{HubsError, SignInError};
use nocterm_hubs_model::HubError;

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
