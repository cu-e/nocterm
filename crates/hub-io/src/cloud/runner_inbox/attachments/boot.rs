//! The original runtime advertises only its own still-live ephemeral recipient.
use super::*;
use futures::future::BoxFuture;

/// Hub receipt time is diagnostic, never boot/head freshness or authority.
#[derive(Clone, Copy, Debug)]
pub struct BootAnnouncement {
    boot: [u8; 16],
    accepted_at_ms: i64,
}
impl BootAnnouncement {
    pub fn boot_id(self) -> [u8; 16] {
        self.boot
    }
    pub fn accepted_at_ms(self) -> i64 {
        self.accepted_at_ms
    }
}
pub(super) fn announce(
    owner: Arc<RunnerCommandAttachments>,
    allocation: AllocationLease,
) -> BoxFuture<'static, Result<BootAnnouncement, CoordinatorError>> {
    let operation = Announcement { owner, allocation };
    Box::pin(async move {
        let result = operation.execute().await;
        result.map_err(|error| retain(error, operation.allocation.clone()))
    })
}
struct Announcement {
    owner: Arc<RunnerCommandAttachments>,
    allocation: AllocationLease,
}
impl Announcement {
    async fn execute(&self) -> Result<BootAnnouncement, CoordinatorError> {
        if self.allocation.bytes() < super::super::super::provider_boot::provider_boot_work_bytes()
            || self.allocation.payloads() == 0
        {
            return Err(CoordinatorError::Overloaded);
        }
        self.require_live()?;
        let status = super::super::super::provider_boot::announcement_body(
            &self.owner.offer.descriptor,
            &self.allocation,
        )
        .map_err(|error| CoordinatorError::Backend(SharedCause::new(error)))?;
        let runner = self.owner.connection.runner().map_err(source_error)?;
        self.require_live()?;
        let accepted_at_ms = runner
            .heartbeat_with_residency(status, Arc::new(self.allocation.clone()))
            .await
            .map_err(backend)?;
        self.require_live()?;
        Ok(BootAnnouncement {
            boot: self.owner.offer.descriptor.descriptor().boot_id(),
            accepted_at_ms,
        })
    }
    fn require_live(&self) -> Result<(), CoordinatorError> {
        let now = self.owner.check_current()?;
        let window = self.owner.offer.descriptor.descriptor().window();
        if self.owner.material.binding() != self.owner.connection.binding()
            || !self
                .owner
                .connection
                .shares_identity(self.owner.material.identity())
            || now.as_millis()
                < u64::try_from(window.issued_at_ms).map_err(|_| CoordinatorError::Expired)?
            || now.as_millis()
                >= u64::try_from(window.expires_at_ms).map_err(|_| CoordinatorError::Expired)?
        {
            return Err(CoordinatorError::Expired);
        }
        Ok(())
    }
}
fn retain(error: CoordinatorError, allocation: AllocationLease) -> CoordinatorError {
    match error {
        CoordinatorError::Backend(cause) => CoordinatorError::Backend(SharedCause::new(
            super::super::super::account_material::retain_error(
                super::super::super::AccountMaterialError::Backend(cause),
                allocation,
            ),
        )),
        other => other,
    }
}
