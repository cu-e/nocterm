//! What a vault record holds, as sealed into one object of the vault collection.
//!
//! The plaintext is `{"v":1,"label":…,"binding":…,"secret":…}`. A record this client cannot
//! read (a newer format version, or a binding of a kind it does not know) is
//! [`Decoded::Unrecognized`]: the caller hides it, never overwrites it and never deletes it.

use nocterm_keys::CollectionKey;
use nocterm_session::Secret;
use nocterm_vault::{CredentialBinding, VaultError};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

use crate::cache::ObjectCopy;

/// The kind of the objects that hold credentials.
pub(super) const KIND: &str = "credential";
/// The only format this client writes and reads.
const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Wire {
    v: u32,
    label: String,
    binding: serde_json::Value,
    secret: String,
}

#[derive(Deserialize)]
struct Head {
    v: u32,
}

/// A credential read from a record.
pub(super) struct Record {
    pub(super) label: String,
    pub(super) binding: CredentialBinding,
    pub(super) secret: Secret,
}

pub(super) enum Decoded {
    Known(Record),
    Unrecognized,
}

/// The sealed text of a credential. The buffer is wiped when dropped.
pub(super) fn encode(
    label: &str,
    binding: &CredentialBinding,
    secret: &Secret,
) -> Result<Zeroizing<Vec<u8>>, serde_json::Error> {
    let mut wire = Wire {
        v: FORMAT,
        label: label.to_owned(),
        binding: serde_json::to_value(binding)?,
        secret: secret.expose().to_owned(),
    };
    let bytes = serde_json::to_vec(&wire);
    wire.secret.zeroize();
    bytes.map(Zeroizing::new)
}

/// Opens a sealed credential object: `Ok(None)` for a tombstone or an object of another kind,
/// and [`VaultError::Authentication`] for one that does not authenticate.
pub(super) fn open_sealed(
    key: &CollectionKey,
    collection_id: &str,
    object_id: &str,
    object: &ObjectCopy,
) -> Result<Option<Decoded>, VaultError> {
    if object.deleted || object.kind != KIND {
        return Ok(None);
    }
    let plain = key
        .open_object(collection_id, object_id, KIND, &object.ciphertext)
        .map_err(|_| VaultError::Authentication)?;
    Ok(Some(decode(&plain)))
}

pub(super) fn decode(plaintext: &[u8]) -> Decoded {
    match serde_json::from_slice::<Head>(plaintext) {
        Ok(Head { v: FORMAT }) => {}
        _ => return Decoded::Unrecognized,
    }
    let Ok(wire) = serde_json::from_slice::<Wire>(plaintext) else {
        return Decoded::Unrecognized;
    };
    match serde_json::from_value::<CredentialBinding>(wire.binding) {
        Ok(binding) => Decoded::Known(Record {
            label: wire.label,
            binding,
            secret: Secret::new(wire.secret),
        }),
        Err(_) => Decoded::Unrecognized,
    }
}

#[cfg(test)]
mod tests;
