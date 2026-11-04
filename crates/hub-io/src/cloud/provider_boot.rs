//! The runner's side of its provider boot: the sticky clock its announcements are checked
//! against and the bound of one announcement. Devices verify a boot and seal a lease to it
//! with the contract device core (`ProviderLease`, ADR 0136).
use nocterm_cloud_model::identity::Timestamp;
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_model::cloud::provider::{SignedBootDescriptor, VerifiedBootDescriptor};

/// SDK proof/body/status generations, ≤580B signed input and ≤512B canonical verify scratch,
/// plus decoded descriptor metadata. No Tonic/H2/TLS/runtime accounting is claimed.
pub fn provider_boot_work_bytes() -> usize {
    nocterm_hub_client::cloud::status_work_bytes()
        + 580
        + 512
        + 3 * (std::mem::size_of::<VerifiedBootDescriptor>() + 2 * 128)
}

/// Why an announcement of the runner's boot was refused.
#[derive(Debug, thiserror::Error)]
pub enum ProviderBootError {
    #[error("the provider boot exceeds its finite allocation or format bound")]
    Capacity,
    #[error("the original provider boot clock was fenced")]
    Clock,
    #[error("the provider boot descriptor could not be encoded")]
    Encoding(#[source] nocterm_hub_model::cloud::provider::ProviderLeaseError),
}

/// A clock that never moves back: a boot window is checked only against it.
#[derive(Default)]
pub(in crate::cloud) struct BootClock {
    last: Option<Timestamp>,
    failed: bool,
}
impl BootClock {
    pub(in crate::cloud) fn observe(
        &mut self,
        now: Timestamp,
    ) -> Result<Timestamp, ProviderBootError> {
        if self.failed
            || now.as_millis() == 0
            || now.as_millis() > i64::MAX as u64
            || self.last.is_some_and(|old| now < old)
        {
            self.failed = true;
            return Err(ProviderBootError::Clock);
        }
        self.last = Some(now);
        Ok(now)
    }
}

/// The canonical bytes of the runner's signed boot, within one announcement's allocation.
pub(in crate::cloud) fn announcement_body(
    signed: &SignedBootDescriptor,
    allocation: &AllocationLease,
) -> Result<Vec<u8>, ProviderBootError> {
    if allocation.bytes() < provider_boot_work_bytes() || allocation.payloads() == 0 {
        return Err(ProviderBootError::Capacity);
    }
    signed
        .canonical_bytes()
        .map_err(ProviderBootError::Encoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boot_clock_that_moves_back_stays_fenced() {
        let mut clock = BootClock::default();
        assert!(clock.observe(Timestamp::from_millis(10)).is_ok());
        assert!(matches!(
            clock.observe(Timestamp::from_millis(9)),
            Err(ProviderBootError::Clock)
        ));
        assert!(clock.observe(Timestamp::from_millis(11)).is_err());
    }
}
