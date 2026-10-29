//! Versioned native binary records. These bytes are never signed hub layouts.
pub(crate) mod archive;
pub(crate) mod head;
pub(crate) mod identity;
pub(crate) mod record;
mod value;

pub(crate) use value::{Checkpoint, Header, Outbox, Position, Receipt, Snapshot};

use nocterm_cloud_model::journal::{
    JournalCipher, JournalCipherContext, JournalError, MAX_STORED_JOURNAL_BYTES,
};
use serde::{Serialize, de::DeserializeOwned};
use zeroize::Zeroizing;

const FORMAT: u8 = 1;

pub(crate) fn seal<T: Serialize>(
    cipher: &dyn JournalCipher,
    context: &JournalCipherContext,
    value: &T,
) -> Result<Vec<u8>, JournalError> {
    let plain =
        Zeroizing::new(postcard::to_allocvec(&(FORMAT, value)).map_err(|_| JournalError::Corrupt)?);
    if plain.len() > MAX_STORED_JOURNAL_BYTES - 512 {
        return Err(JournalError::Capacity);
    }
    let bytes = cipher.seal(context, &plain)?;
    if bytes.len() > plain.len().saturating_add(512) {
        return Err(JournalError::Capacity);
    }
    bounded(&bytes)?;
    Ok(bytes)
}

pub(crate) fn open<T: DeserializeOwned>(
    cipher: &dyn JournalCipher,
    context: &JournalCipherContext,
    bytes: &[u8],
) -> Result<T, JournalError> {
    bounded(bytes)?;
    let plain = Zeroizing::new(cipher.open(context, bytes)?);
    bounded(&plain)?;
    if plain.len() > bytes.len() {
        return Err(JournalError::Corrupt);
    }
    let ((format, value), trailing) =
        postcard::take_from_bytes::<(u8, T)>(&plain).map_err(|_| JournalError::Corrupt)?;
    if format != FORMAT || !trailing.is_empty() {
        return Err(JournalError::Corrupt);
    }
    Ok(value)
}

fn bounded(bytes: &[u8]) -> Result<(), JournalError> {
    if bytes.is_empty() || bytes.len() > MAX_STORED_JOURNAL_BYTES {
        Err(JournalError::Capacity)
    } else {
        Ok(())
    }
}
