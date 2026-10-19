//! Format callbacks run outside census. Persistent cache handles expose only the
//! domain-owned immutable storage graph and physical header lifetime.
use super::LegacyRetentionSource;
use crate::{
    history::{
        FormatSize, HistoryFraming, HistoryShape, HistoryView, LegacySizing, LegacySizingError,
        SavedPrompt,
    },
    model::{
        json::{
            deferred::{DecodeLedger, JsonDecodeError, ledger::Reservation},
            lifetime::Shared,
        },
        retention::{
            CensusPermit, RetentionAuthority, RetentionCell, RetentionError,
            graph::{Count, OwnerGraph, sealed},
        },
    },
    thread::Entry,
};
use std::sync::{Arc, OnceLock, Weak};
struct Lease {
    retention: RetentionCell,
    source: LegacyRetentionSource,
    bytes: usize,
    allocation: Option<Reservation>,
}
struct FormatOwner<T: LegacySizing> {
    value: T,
    lease: Shared<Lease>,
    identity: OnceLock<LegacyIdentity>,
}
/// Original physical grammar identity, with no adapter callback on count/cleanup.
/// Weak pins its real header; the lease owns the actual shared row storage.
#[derive(Clone)]
pub struct LegacyIdentity {
    _header: Weak<dyn LegacySizing>,
    lease: Shared<Lease>,
}
impl LegacyIdentity {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        self._header.ptr_eq(&other._header)
    }
}
/// Owning facade keeps the genuine header lease outside the trait Arc's Drop.
/// No raw owning Arc escapes; borrowing the grammar cannot extend that Drop.
#[derive(Clone)]
pub struct LegacyFormat {
    value: Arc<dyn LegacySizing>,
    identity: LegacyIdentity,
}
impl AsRef<dyn LegacySizing> for LegacyFormat {
    fn as_ref(&self) -> &(dyn LegacySizing + 'static) {
        self.value.as_ref()
    }
}
impl std::ops::Deref for LegacyFormat {
    type Target = dyn LegacySizing;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref()
    }
}
impl sealed::Source for LegacyFormat {}
impl OwnerGraph for LegacyFormat {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.identity.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.identity.cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.identity.adopt(publication);
    }
}
pub fn retain_legacy<T: LegacySizing>(
    authority: &RetentionAuthority,
    value: T,
    source: LegacyRetentionSource,
) -> LegacyFormat {
    build(authority, value, source, None)
}
pub fn retain_legacy_admitted<T: LegacySizing>(
    ledger: &DecodeLedger,
    value: T,
    source: LegacyRetentionSource,
) -> Result<LegacyFormat, JsonDecodeError> {
    let allocation = ledger.reserve(inline::<T>())?;
    Ok(build(ledger.authority(), value, source, Some(allocation)))
}
fn inline<T: LegacySizing>() -> usize {
    crate::model::allocations::shared_inline_bytes::<FormatOwner<T>>()
        + crate::model::allocations::shared_inline_bytes::<Lease>()
}
fn build<T: LegacySizing>(
    authority: &RetentionAuthority,
    value: T,
    source: LegacyRetentionSource,
    allocation: Option<Reservation>,
) -> LegacyFormat {
    let lease = Shared::new(Lease {
        retention: RetentionCell::new(authority),
        source,
        bytes: inline::<T>(),
        allocation,
    });
    let owner = Arc::new(FormatOwner {
        value,
        lease: lease.clone(),
        identity: OnceLock::new(),
    });
    let erased: Arc<dyn LegacySizing> = owner.clone();
    let identity = owner
        .identity
        .get_or_init(|| LegacyIdentity {
            _header: Arc::downgrade(&erased),
            lease,
        })
        .clone();
    LegacyFormat {
        value: erased,
        identity,
    }
}
impl<T: LegacySizing> LegacySizing for FormatOwner<T> {
    fn retained_heap_bytes(&self) -> Result<usize, LegacySizingError> {
        self.value
            .retained_heap_bytes()?
            .checked_add(self.lease.bytes)
            .ok_or(LegacySizingError::ArithmeticOverflow)
    }
    fn retention(&self) -> Option<&LegacyIdentity> {
        self.identity.get()
    }
    fn entry(&self, row: &Entry) -> Result<FormatSize, LegacySizingError> {
        self.value.entry(row)
    }
    fn prompt(&self, prompt: &SavedPrompt) -> Result<FormatSize, LegacySizingError> {
        self.value.prompt(prompt)
    }
    fn framing(
        &self,
        frame: HistoryFraming<'_>,
        shape: HistoryShape,
    ) -> Result<FormatSize, LegacySizingError> {
        self.value.framing(frame, shape)
    }
    fn measure(&self, candidate: HistoryView<'_>) -> Result<FormatSize, LegacySizingError> {
        self.value.measure(candidate)
    }
    fn managed_entry(
        &self,
        row: &crate::history::physical::RowSlot,
    ) -> Result<FormatSize, LegacySizingError> {
        self.value.managed_entry(row)
    }
    fn managed_prompt(
        &self,
        prompt: &crate::history::physical::PromptSlot,
    ) -> Result<FormatSize, LegacySizingError> {
        self.value.managed_prompt(prompt)
    }
    fn managed_measure(
        &self,
        snapshot: &crate::history::physical::SnapshotOwner,
    ) -> Result<FormatSize, LegacySizingError> {
        self.value.managed_measure(snapshot)
    }
}
impl sealed::Source for LegacyIdentity {}
impl OwnerGraph for LegacyIdentity {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if count.enter(&self.lease.retention, self.lease.bytes)? {
            self.lease.source.count(count)?;
            if let Some(allocation) = &self.lease.allocation {
                allocation.validate_pending(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.lease.retention.clear_owned(permit) {
            self.lease.source.cleanup(permit);
            if let Some(allocation) = &self.lease.allocation {
                allocation.cleanup_pending(permit);
            }
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        if publication.enter(&self.lease.retention) {
            self.lease.source.adopt(publication);
            if let Some(allocation) = &self.lease.allocation {
                allocation.adopt_under(publication);
            }
        }
    }
}

/// The native writer recovers its original private format policy; no protocol
/// type crosses the domain API and the caller cannot substitute another owner.
pub fn legacy_value<T: LegacySizing>(provider: &dyn LegacySizing) -> Option<&T> {
    (provider as &dyn std::any::Any)
        .downcast_ref::<FormatOwner<T>>()
        .map(|owner| &owner.value)
}
