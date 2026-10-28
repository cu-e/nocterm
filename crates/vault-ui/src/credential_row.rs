//! How a credential is described in the Vault page's list.
use nocterm_ssh_keys::PublicKeyText;
use nocterm_vault::{CredentialBinding, CredentialInfo};

/// The second line of a credential row: a key shows its fingerprint, which is
/// what other tools print for it; everything else shows its ID. A credential of a group
/// also names the group and whether the member may only read it.
pub(crate) fn describe(record: &CredentialInfo) -> String {
    let what = match &record.binding {
        CredentialBinding::PrivateKey { public_key } => match PublicKeyText::parse(public_key) {
            Ok(public) => format!("SSH key \u{b7} {}", public.fingerprint()),
            Err(_) => format!("SSH key \u{b7} {}", record.id),
        },
        _ => record.id.to_string(),
    };
    match &record.group {
        Some(group) if group.writable => format!("{what} \u{b7} Group {}", group.name),
        Some(group) => format!("{what} \u{b7} Group {} \u{b7} read-only", group.name),
        None => what,
    }
}

#[cfg(test)]
mod tests;
