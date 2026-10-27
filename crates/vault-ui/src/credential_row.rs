//! How a credential is described in the Vault page's list.
use nocterm_ssh_keys::PublicKeyText;
use nocterm_vault::{CredentialBinding, CredentialInfo};

/// The second line of a credential row: a key shows its fingerprint, which is
/// what other tools print for it; everything else shows its ID.
pub(crate) fn describe(record: &CredentialInfo) -> String {
    match &record.binding {
        CredentialBinding::PrivateKey { public_key } => match PublicKeyText::parse(public_key) {
            Ok(public) => format!("SSH key \u{b7} {}", public.fingerprint()),
            Err(_) => format!("SSH key \u{b7} {}", record.id),
        },
        _ => record.id.to_string(),
    }
}

#[cfg(test)]
mod tests;
