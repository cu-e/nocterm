//! The sole public SDK supplies both native metadata purposes and their codecs.
use super::CloudRunnerIdentity;
use nocterm_cloud_model::journal::{
    JournalCatalogBinding, JournalCatalogCipher, JournalCatalogContext, JournalChatHeadCipher,
    JournalChatHeadContext, JournalError,
};
use nocterm_core::SharedCause;
use nocterm_hub_model::cloud::{
    CloudNamespace, CloudTextId, RunnerCoordinate,
    account_catalog::{AccountCatalogCoordinate, MAX_ACCOUNT_CATALOG_BYTES},
    native_chat_head::{
        MAX_NATIVE_CHAT_HEAD_BYTES, MAX_NATIVE_CHAT_HEAD_CIPHERTEXT_BYTES, NativeChatHeadCoordinate,
    },
};
use nocterm_keys::cloud::{NativeCatalogKey, NativeChatHeadKey};
use std::{error::Error, fmt, sync::Arc};

/// Bound to one real persisted identity and full native account/R pins. This
/// metadata facade cannot encrypt transcript content or install a content key.
#[derive(Clone)]
pub struct RunnerMetadataCrypto {
    identity: Arc<CloudRunnerIdentity>,
}
impl RunnerMetadataCrypto {
    pub fn new(identity: Arc<CloudRunnerIdentity>) -> Self {
        Self { identity }
    }
    fn catalog(
        &self,
        context: &JournalCatalogContext,
    ) -> Result<AccountCatalogCoordinate, JournalError> {
        if context.binding() != self.identity.binding() {
            return Err(JournalError::Fenced);
        }
        let binding = context.binding();
        AccountCatalogCoordinate::new(
            namespace(binding)?,
            *binding.scope().account().key().as_bytes(),
            runner(binding),
            context.revision(),
        )
        .map_err(crypto_error)
    }
    fn head(
        &self,
        context: &JournalChatHeadContext,
    ) -> Result<NativeChatHeadCoordinate, JournalError> {
        let binding = self.identity.binding();
        if !binding.contains(context.stream()) {
            return Err(JournalError::Fenced);
        }
        NativeChatHeadCoordinate::new(
            namespace(binding)?,
            *binding.scope().account().key().as_bytes(),
            runner(binding),
            *context.stream().chat().as_bytes(),
            context.writer().map(|value| value.get()),
            context.revision(),
        )
        .map_err(crypto_error)
    }
}
impl JournalCatalogCipher for RunnerMetadataCrypto {
    fn seal(
        &self,
        context: &JournalCatalogContext,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        if plaintext.is_empty() || plaintext.len() > MAX_ACCOUNT_CATALOG_BYTES {
            return Err(JournalError::Capacity);
        }
        let coordinate = self.catalog(context)?;
        self.identity
            .with_keys(|keys| {
                NativeCatalogKey::for_runner(keys.encryption(), &coordinate)
                    .and_then(|key| key.seal_catalog(plaintext))
            })
            .map_err(crypto_error)?
            .map_err(crypto_error)
    }
    fn open(
        &self,
        context: &JournalCatalogContext,
        sealed: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        if sealed.len() <= 42 || sealed.len() > MAX_ACCOUNT_CATALOG_BYTES + 42 {
            return Err(JournalError::Capacity);
        }
        let coordinate = self.catalog(context)?;
        let mut opened = self
            .identity
            .with_keys(|keys| {
                NativeCatalogKey::for_runner(keys.encryption(), &coordinate)
                    .and_then(|key| key.open_catalog(sealed))
            })
            .map_err(crypto_error)?
            .map_err(crypto_error)?;
        // Move the existing allocation into the native zeroizing owner instead
        // of creating a second full plaintext copy during this port handoff.
        Ok(std::mem::take(&mut *opened))
    }
}
impl JournalChatHeadCipher for RunnerMetadataCrypto {
    fn seal(
        &self,
        context: &JournalChatHeadContext,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        if plaintext.is_empty() || plaintext.len() > MAX_NATIVE_CHAT_HEAD_BYTES {
            return Err(JournalError::Capacity);
        }
        let coordinate = self.head(context)?;
        self.identity
            .with_keys(|keys| {
                NativeChatHeadKey::for_runner(keys.encryption(), &coordinate)
                    .and_then(|key| key.seal_head(plaintext))
            })
            .map_err(crypto_error)?
            .map_err(crypto_error)
    }
    fn open(
        &self,
        context: &JournalChatHeadContext,
        sealed: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        if sealed.len() <= 42 || sealed.len() > MAX_NATIVE_CHAT_HEAD_CIPHERTEXT_BYTES {
            return Err(JournalError::Capacity);
        }
        let coordinate = self.head(context)?;
        let mut opened = self
            .identity
            .with_keys(|keys| {
                NativeChatHeadKey::for_runner(keys.encryption(), &coordinate)
                    .and_then(|key| key.open_head(sealed))
            })
            .map_err(crypto_error)?
            .map_err(crypto_error)?;
        Ok(std::mem::take(&mut *opened))
    }
}
impl fmt::Debug for RunnerMetadataCrypto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerMetadataCrypto")
            .field("binding", self.identity.binding())
            .finish_non_exhaustive()
    }
}
fn crypto_error(error: impl Error + Send + Sync + 'static) -> JournalError {
    JournalError::Storage(SharedCause::new(error))
}
pub(super) fn namespace(binding: &JournalCatalogBinding) -> Result<CloudNamespace, JournalError> {
    Ok(CloudNamespace {
        hub_instance_id: CloudTextId::new(binding.scope().hub().instance().as_str())
            .map_err(crypto_error)?,
        account_id: CloudTextId::new(binding.scope().account().id().as_str())
            .map_err(crypto_error)?,
    })
}
pub(super) fn runner(binding: &JournalCatalogBinding) -> RunnerCoordinate {
    RunnerCoordinate {
        id: *binding.runner().as_bytes(),
        key_epoch: binding.epoch().get(),
    }
}

#[cfg(test)]
mod tests;
