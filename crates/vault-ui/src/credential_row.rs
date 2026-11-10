//! How a credential is described in the Vault page's list.
use nocterm_ssh_keys::PublicKeyText;
use nocterm_vault::{CredentialBinding, CredentialInfo};

/// The second line of a credential row: a key shows its fingerprint, which is
/// what other tools print for it; a password how many servers it signs in to, which are
/// listed under it; a key passphrase the key file it opens.
pub(crate) fn describe(record: &CredentialInfo) -> String {
    match &record.binding {
        CredentialBinding::PrivateKey { public_key } => match PublicKeyText::parse(public_key) {
            Ok(public) => format!("SSH key \u{b7} {}", public.fingerprint()),
            Err(_) => format!("SSH key \u{b7} {}", record.id),
        },
        CredentialBinding::Password { targets } => match targets.len() {
            0 => "Password \u{b7} signs in to no server yet".to_owned(),
            1 => "Password \u{b7} signs in to 1 server".to_owned(),
            n => format!("Password \u{b7} shared by {n} servers"),
        },
        CredentialBinding::KeyPassphrase { path } => {
            format!("Key passphrase \u{b7} {}", path.display())
        }
    }
}

#[cfg(test)]
mod tests;
