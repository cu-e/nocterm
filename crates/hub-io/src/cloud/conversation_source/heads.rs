//! Fresh account/R nonce witnesses; hub replay cursors never establish authority.
use super::{capture::NativeSource, clock::SourceClock, sdk_error};
use nocterm_cloud_model::{
    identity::*,
    journal::{CurrentChatHead, HeadAnchor},
    projection::{ProjectionError, RecordKind},
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use nocterm_hub_client::cloud::{DeviceHeadProbes, ResidentHeadWitness};
use nocterm_hub_model::cloud::head::{
    ChatHead, HeadProbeKeys, HeadProbeScope, HeadRecordAnchor, HeadTarget, HeadValue,
    MAX_HEAD_CATALOG_CHATS, MAX_HEAD_CATALOG_PAGE,
};
use std::sync::Arc;

fn sdk_work_bytes() -> usize {
    nocterm_hub_client::cloud::head_query_work_bytes()
        + nocterm_hub_client::cloud::head_reply_work_bytes()
        + nocterm_hub_client::cloud::head_decoded_metadata_bytes()
}
pub(super) fn work_bytes() -> usize {
    sdk_work_bytes() + 32 * 1024
}
pub(super) struct ProbeWork {
    allocation: AllocationLease,
    slices: Arc<super::work::WorkSlices>,
}
impl ProbeWork {
    pub(super) fn new(allocation: AllocationLease) -> Result<Self, ProjectionError> {
        require(&allocation)?;
        let slices = super::work::WorkSlices::new(allocation.clone(), sdk_work_bytes())?;
        Ok(Self { allocation, slices })
    }
}
#[derive(Clone)]
pub(super) struct FreshHead {
    pub revision: u64,
    pub root: Fingerprint,
    pub head: CurrentChatHead,
}
/// Every page answers its own fresh one-shot OS nonce under the same root/cursor.
/// Missing/pruned/new-writer conditions never become an empty synthetic chat.
pub(super) async fn current(
    source: &Arc<NativeSource>,
    clock: &SourceClock,
    work: &ProbeWork,
) -> Result<Allocated<FreshHead>, ProjectionError> {
    require(&work.allocation)?;
    let mut after = None;
    let mut root = None;
    let mut revision = None;
    let mut catalog_head = None;
    let maximum_pages = (MAX_HEAD_CATALOG_CHATS as usize).div_ceil(MAX_HEAD_CATALOG_PAGE) + 1;
    for _ in 0..maximum_pages {
        let witness = probe(
            source,
            clock,
            HeadTarget::Catalog {
                after: after.clone(),
            },
            work,
        )
        .await?;
        let HeadValue::Catalog(page) = &witness.witness().witness().value else {
            return Err(ProjectionError::InvalidRecord(RecordKind::Head));
        };
        if root.is_some_and(|root| root != page.digest)
            || revision.is_some_and(|revision| revision != page.revision)
        {
            return Err(ProjectionError::Rollback);
        }
        root = Some(page.digest);
        revision = Some(page.revision);
        if let Some(head) = page
            .chats
            .iter()
            .find(|head| &head.chat_id == source.material().request().stream.chat().as_bytes())
        {
            catalog_head = Some(native(source, head)?);
            break;
        }
        let next = page.next_cursor();
        if next.is_none() {
            return Err(ProjectionError::Unavailable);
        }
        if next == after {
            return Err(ProjectionError::Discontinuous);
        }
        after = next;
    }
    let old = catalog_head.ok_or(ProjectionError::Capacity)?;
    let request = source.material().request();
    let witness = probe(
        source,
        clock,
        HeadTarget::Chat {
            chat_id: *request.stream.chat().as_bytes(),
            chat_key_epoch: request.epoch.get(),
        },
        work,
    )
    .await?;
    let HeadValue::Chat(head) = &witness.witness().witness().value else {
        return Err(ProjectionError::InvalidRecord(RecordKind::Head));
    };
    let head = native(source, head)?;
    require_monotonic(&old, &head)?;
    source.check()?;
    Ok(Allocated::new(
        FreshHead {
            revision: revision.ok_or(ProjectionError::InvalidRecord(RecordKind::Head))?,
            root: Fingerprint::from_bytes(
                root.ok_or(ProjectionError::InvalidRecord(RecordKind::Head))?,
            ),
            head,
        },
        work.allocation.clone(),
    ))
}
pub(super) async fn probe(
    source: &Arc<NativeSource>,
    clock: &SourceClock,
    target: HeadTarget,
    work: &ProbeWork,
) -> Result<ResidentHeadWitness, ProjectionError> {
    require(&work.allocation)?;
    source.check()?;
    let port: DeviceHeadProbes = source
        .cloud()
        .head_probes_with_residency(Arc::new(work.slices.reserve(sdk_work_bytes())?))
        .map_err(sdk_error)?;
    let request = source.material().request();
    let scope = source.with_keyset(|keys| {
        Ok(HeadProbeScope {
            namespace: source.cloud().namespace().clone(),
            runner: nocterm_hub_model::cloud::RunnerCoordinate {
                id: *request.stream.runner().as_bytes(),
                key_epoch: request.stream.runner_epoch().get(),
            },
            keys: HeadProbeKeys {
                account: keys.public_key().to_bytes(),
                runner_encryption: request.pins.encryption,
                runner_signing: request.pins.signing,
            },
        })
    })?;
    let now = clock.millis()?;
    let prepared = source.with_keyset(|keys| port.prepare(keys, scope, target, now))?;
    source.check()?;
    let completed = port.request(prepared).await.map_err(sdk_error)?;
    source.check()?;
    let now = clock.millis()?;
    let witness = source.with_keyset(|keys| completed.open_resident(keys, now))?;
    source.check()?;
    Ok(witness)
}
fn require(work: &AllocationLease) -> Result<(), ProjectionError> {
    if work.payloads() == 0 || work.bytes() < work_bytes() {
        Err(ProjectionError::Capacity)
    } else {
        Ok(())
    }
}
fn native(source: &NativeSource, value: &ChatHead) -> Result<CurrentChatHead, ProjectionError> {
    let request = source.material().request();
    if value.chat_id != *request.stream.chat().as_bytes()
        || value.chat_key_epoch != request.epoch.get()
    {
        return Err(ProjectionError::WrongSource);
    }
    crate::cloud::head_mapping::PinnedHeadMapper::for_stream(&request.stream).head(value)
}
pub(super) fn anchor(
    source: &NativeSource,
    value: &HeadRecordAnchor,
) -> Result<HeadAnchor, ProjectionError> {
    let stream = &source.material().request().stream;
    crate::cloud::head_mapping::PinnedHeadMapper::for_stream(stream).anchor(stream, value)
}
pub(super) fn require_monotonic(
    previous: &CurrentChatHead,
    next: &CurrentChatHead,
) -> Result<(), ProjectionError> {
    if previous.stream() != next.stream() || previous.key_epoch() != next.key_epoch() {
        return Err(ProjectionError::WrongSource);
    }
    if next.facts().control < previous.facts().control
        || (previous.facts().deleted && !next.facts().deleted)
        || next.facts().writer < previous.facts().writer
    {
        return Err(ProjectionError::Rollback);
    }
    match (previous.facts().head.as_ref(), next.facts().head.as_ref()) {
        (Some(old), Some(new))
            if new.position().event() < old.position().event()
                || (new.position().event() == old.position().event() && new != old) =>
        {
            Err(ProjectionError::Rollback)
        }
        (Some(_), None) => Err(ProjectionError::Rollback),
        _ => Ok(()),
    }
}
