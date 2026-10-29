//! Account-pinned consumption into a trusted native receiver; no plaintext getter.
#[cfg(test)]
mod tests;
use super::*;
use nocterm_cloud_model::{
    coordinator::{ProviderCredentialReceiver, ProviderCredentialSource},
    journal::JournalCatalogBinding,
};
use nocterm_keys::PublicKey;

/// A configured original account root is mandatory. A key from a ciphertext,
/// directory reply, provider setting or child process can never replace it.
pub struct RunnerProviderCredentialSource {
    attachments: Arc<RunnerCommandAttachments>,
    account: PublicKey,
}
impl RunnerProviderCredentialSource {
    pub fn new(
        attachments: Arc<RunnerCommandAttachments>,
        account: [u8; 32],
    ) -> Result<Self, CoordinatorError> {
        if <[u8; 32]>::from(Sha256::digest(account))
            != *attachments
                .material
                .binding()
                .scope()
                .account()
                .key()
                .as_bytes()
        {
            return Err(CoordinatorError::StaleSource);
        }
        let account = PublicKey::from_bytes(&account).map_err(backend)?;
        Ok(Self {
            attachments,
            account,
        })
    }
}
impl ProviderCredentialSource for RunnerProviderCredentialSource {
    fn binding(&self) -> &JournalCatalogBinding {
        self.attachments.material.binding()
    }
    fn check_current(&self) -> Result<(), CoordinatorError> {
        self.attachments.check_current().map(|_| ())
    }
    fn consume(
        &self,
        command: &CommandAcknowledgement,
        offer: &ProviderOffer,
        receiver: &mut dyn ProviderCredentialReceiver,
    ) -> Result<(), CoordinatorError> {
        self.check_current()?;
        if !self.binding().contains(command.stream()) {
            return Err(CoordinatorError::StaleSource);
        }
        let staged = self
            .attachments
            .take_provider(command, offer)?
            .ok_or(CoordinatorError::NeedsProviderAccess)?;
        // Consumption is one-shot. A failed authentication/receiver/currentness
        // check needs a new signed grant; reinserting into another generation
        // could reopen an accepted capsule or replace a newer observation.
        staged.check_current()?;
        let now = self.attachments.check_current()?;
        open_into(
            &self.attachments.boot,
            &self.account,
            &staged.context,
            &staged.capsule,
            now,
            staged._allocation.clone(),
            receiver,
        )?;
        self.check_current()
    }
}

// The existing stage reservation is 2 * original payload + 32 KiB. Its fixed
// allowance covers the extra <=16 KiB plaintext/copy overlap and SDK context
// work below. The original capsule/offer and retained key share that SAME real
// allocation; configured transport/connection baseline is owned separately.
const CONTEXT_BYTES: usize = 1024;
const ACCOUNT_RECIPIENT_BYTES: usize = 64;
const SDK_CONTEXT_WORK: usize = 512
    + 2 * CONTEXT_BYTES
    + 2 * (CONTEXT_BYTES + ACCOUNT_RECIPIENT_BYTES)
    + 2 * (CONTEXT_BYTES + ACCOUNT_RECIPIENT_BYTES)
    + 128;

fn open_into(
    boot: &RunnerBootKey,
    account: &PublicKey,
    context: &ProviderLeaseContext,
    capsule: &[u8],
    now: Timestamp,
    allocation: AllocationLease,
    receiver: &mut dyn ProviderCredentialReceiver,
) -> Result<(), CoordinatorError> {
    let required = capsule
        .len()
        .checked_mul(3)
        .and_then(|bytes| bytes.checked_add(SDK_CONTEXT_WORK))
        .ok_or(CoordinatorError::Overloaded)?;
    if allocation.bytes() < required {
        return Err(CoordinatorError::Overloaded);
    }
    let retained = allocation.clone();
    let opened = boot
        .open_provider_lease(
            account,
            context,
            capsule,
            i64::try_from(now.as_millis()).map_err(|_| CoordinatorError::Expired)?,
        )
        .map_err(backend)?;
    let result = receiver.install(opened.secret(), allocation);
    drop(opened);
    drop(retained);
    result
}
