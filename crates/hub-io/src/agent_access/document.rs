//! The CA document of the personal `agent_ssh_trust` collection: the seed of the CA key and the
//! public keys of the CAs it replaced.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use nocterm_ssh_keys::ca::UserCa;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The object id of the CA document.
pub(super) const CA_OBJECT: &str = "ca";
/// Newer than any document this client writes.
const FORMAT: u8 = 1;
/// Only a handful of replaced CAs are kept; older ones no longer have lines to find.
const MAX_RETIRED: usize = 8;

/// Why a stored document cannot be used.
#[derive(Debug, thiserror::Error)]
pub(super) enum DocumentError {
    #[error("the CA document is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the CA document has another format")]
    Format,
    #[error("the CA document holds a key that is not 32 bytes")]
    Key,
    #[error("the CA key cannot be rebuilt from its seed")]
    Seed(#[source] nocterm_ssh_keys::KeyImportError),
}

#[derive(Serialize, Deserialize)]
struct Stored {
    format: u8,
    seed: String,
    created_at_ms: i64,
    #[serde(default)]
    retired: Vec<String>,
}

/// The CA of the account and the keys it replaced.
pub(super) struct CaDocument {
    ca: UserCa,
    created_at_ms: i64,
    retired: Vec<[u8; 32]>,
}

fn key_of(text: &str) -> Result<[u8; 32], DocumentError> {
    STANDARD
        .decode(text)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or(DocumentError::Key)
}

impl CaDocument {
    pub(super) fn new(seed: [u8; 32], now_ms: i64) -> Result<Self, DocumentError> {
        Ok(Self {
            ca: UserCa::from_seed(seed).map_err(DocumentError::Seed)?,
            created_at_ms: now_ms,
            retired: Vec::new(),
        })
    }

    /// The document with a new CA; the current key is remembered so its lines are still found.
    pub(super) fn replaced(&self, seed: [u8; 32], now_ms: i64) -> Result<Self, DocumentError> {
        let mut retired = self.retired.clone();
        retired.insert(0, self.ca.public_key());
        retired.truncate(MAX_RETIRED);
        Ok(Self {
            ca: UserCa::from_seed(seed).map_err(DocumentError::Seed)?,
            created_at_ms: now_ms,
            retired,
        })
    }

    pub(super) fn ca(&self) -> &UserCa {
        &self.ca
    }

    pub(super) fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }

    pub(super) fn retired(&self) -> &[[u8; 32]] {
        &self.retired
    }

    pub(super) fn encode(&self) -> Result<Zeroizing<Vec<u8>>, DocumentError> {
        let stored = Stored {
            format: FORMAT,
            seed: STANDARD.encode(self.ca.seed()),
            created_at_ms: self.created_at_ms,
            retired: self
                .retired
                .iter()
                .map(|key| STANDARD.encode(key))
                .collect(),
        };
        serde_json::to_vec(&stored)
            .map(Zeroizing::new)
            .map_err(DocumentError::Json)
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, DocumentError> {
        let stored: Stored = serde_json::from_slice(bytes).map_err(DocumentError::Json)?;
        if stored.format != FORMAT {
            return Err(DocumentError::Format);
        }
        let seed = Zeroizing::new(key_of(&stored.seed)?);
        Ok(Self {
            ca: UserCa::from_seed(*seed).map_err(DocumentError::Seed)?,
            created_at_ms: stored.created_at_ms,
            retired: stored
                .retired
                .iter()
                .map(|key| key_of(key))
                .collect::<Result<_, _>>()?,
        })
    }
}
