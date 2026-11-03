//! A provider lease a device hands to the runner of a chat (ADR 0126).
//!
//! The capsule is the provider credential already sealed on the device to the runner's boot
//! key; this value never holds the credential itself. Its `Debug` shows sizes only.

use std::fmt;

use super::{ProjectionError, RecordKind};
use crate::identity::Timestamp;

/// The most models one lease can name.
const MAX_MODELS: usize = 128;
/// Canonical lease context and sealed capsule stay inside the wire bounds.
const MAX_CONTEXT_BYTES: usize = 1024;
const MAX_CAPSULE_BYTES: usize = 16 * 1024 + 74;

/// One lease of the provider credential for a chat.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderAccess {
    grant: [u8; 16],
    epoch: u64,
    context: Vec<u8>,
    models: Vec<String>,
    capsule: Vec<u8>,
    expires: Timestamp,
}

impl ProviderAccess {
    /// `grant` names the lease of one provider in one chat and `epoch` grows with every
    /// renewal, so a newer lease replaces an older one on the runner. `context` is the
    /// canonical lease context the capsule was sealed under.
    pub fn new(
        grant: [u8; 16],
        epoch: u64,
        context: Vec<u8>,
        models: Vec<String>,
        capsule: Vec<u8>,
        expires: Timestamp,
    ) -> Result<Self, ProjectionError> {
        if epoch == 0
            || context.is_empty()
            || context.len() > MAX_CONTEXT_BYTES
            || capsule.is_empty()
            || capsule.len() > MAX_CAPSULE_BYTES
            || models.is_empty()
            || models.len() > MAX_MODELS
            || expires.as_millis() == 0
        {
            return Err(ProjectionError::InvalidRecord(RecordKind::ProviderAccess));
        }
        Ok(Self {
            grant,
            epoch,
            context,
            models,
            capsule,
            expires,
        })
    }

    pub fn grant(&self) -> [u8; 16] {
        self.grant
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn context(&self) -> &[u8] {
        &self.context
    }

    pub fn models(&self) -> &[String] {
        &self.models
    }

    pub fn capsule(&self) -> &[u8] {
        &self.capsule
    }

    pub fn expires(&self) -> Timestamp {
        self.expires
    }

    /// Bytes the lease holds while it waits to be sent.
    pub fn resident_bytes(&self) -> usize {
        self.context.len() + self.capsule.len() + self.models.iter().map(String::len).sum::<usize>()
    }
}

impl fmt::Debug for ProviderAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderAccess")
            .field("epoch", &self.epoch)
            .field("models", &self.models.len())
            .field("capsule_bytes", &self.capsule.len())
            .field("expires", &self.expires)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access() -> ProviderAccess {
        ProviderAccess::new(
            [1; 16],
            2,
            vec![9; 8],
            vec!["model".into()],
            b"sealed-bytes".to_vec(),
            Timestamp::from_millis(5),
        )
        .unwrap()
    }

    #[test]
    fn debug_shows_sizes_and_never_the_capsule() {
        let shown = format!("{:?}", access());
        assert!(shown.contains("capsule_bytes"));
        assert!(!shown.contains("sealed-bytes"));
    }

    #[test]
    fn an_empty_or_unbounded_lease_is_refused() {
        let build = |epoch: u64, capsule: Vec<u8>, models: Vec<String>| {
            ProviderAccess::new(
                [1; 16],
                epoch,
                vec![1],
                models,
                capsule,
                Timestamp::from_millis(5),
            )
        };
        assert!(build(0, vec![1], vec!["m".into()]).is_err());
        assert!(build(1, Vec::new(), vec!["m".into()]).is_err());
        assert!(build(1, vec![1], Vec::new()).is_err());
        assert!(build(1, vec![0; MAX_CAPSULE_BYTES + 1], vec!["m".into()]).is_err());
    }
}
