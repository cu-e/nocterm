//! Explicit finite FIFO uploads. Network waits never retain the native write gate.
use super::{services::CoordinatorServices, supervisor::TaskClass};
use nocterm_cloud_model::{authority::StreamBinding, coordinator::CoordinatorError, journal::*};
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use tokio::sync::Mutex as AsyncMutex;

/// Native bootstrap supplies actual transport, crypto and finite work policy.
/// No worker lives on an idle chat; weak lanes survive only an actual upload.
pub struct CoordinatorPublication {
    snapshots: Arc<dyn JournalSnapshotPublication>,
    reader: Arc<dyn OutboxPublicationReader>,
    deliveries: Arc<dyn OutboxDeliveryJournal>,
    transport: Arc<dyn OutboxPublicationTransport>,
    work: Arc<dyn PublicationWorkBudget>,
    limits: JournalReadLimits,
    lanes: Mutex<BTreeMap<StreamBinding, Weak<AsyncMutex<()>>>>,
    concurrency: usize,
}
impl CoordinatorPublication {
    pub fn new(
        snapshots: Arc<dyn JournalSnapshotPublication>,
        reader: Arc<dyn OutboxPublicationReader>,
        deliveries: Arc<dyn OutboxDeliveryJournal>,
        transport: Arc<dyn OutboxPublicationTransport>,
        work: Arc<dyn PublicationWorkBudget>,
        limits: JournalReadLimits,
        concurrency: usize,
    ) -> Result<Self, JournalError> {
        if concurrency == 0 || concurrency > 256 {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            snapshots,
            reader,
            deliveries,
            transport,
            work,
            limits,
            lanes: Mutex::new(BTreeMap::new()),
            concurrency,
        })
    }
    fn lane(&self, stream: &StreamBinding) -> Result<Arc<AsyncMutex<()>>, JournalError> {
        let mut lanes = self.lanes.lock();
        lanes.retain(|_, lane| lane.strong_count() > 0);
        if let Some(lane) = lanes.get(stream).and_then(Weak::upgrade) {
            return Ok(lane);
        }
        if lanes.len() >= self.concurrency {
            return Err(JournalError::Capacity);
        }
        let lane = Arc::new(AsyncMutex::new(()));
        lanes.insert(stream.clone(), Arc::downgrade(&lane));
        Ok(lane)
    }
}
pub(super) async fn start(
    services: Arc<CoordinatorServices>,
    stream: StreamBinding,
    compact: bool,
) -> Result<Option<ResidentSnapshot>, CoordinatorError> {
    let owner = services
        .publication()
        .cloned()
        .ok_or(JournalError::UnsupportedFormat)?;
    let tasks = services.tasks().clone();
    tasks
        .submit(
            TaskClass::Native,
            Box::pin(async move {
                let lane = owner.lane(&stream)?;
                let _serial = lane.lock().await;
                services.source().check_current()?;
                let target = if compact {
                    let lease = current(&services, &stream).await?;
                    let snapshots = owner.snapshots.clone();
                    let candidate = services.io().run(move |_| snapshots.prepare(lease)).await?;
                    services.source().check_current()?;
                    Some(candidate)
                } else {
                    let snapshots = owner.snapshots.clone();
                    let lookup = stream.clone();
                    services
                        .io()
                        .run(move |_| snapshots.pending(lookup))
                        .await?
                };
                flush(&services, &owner, &stream, target).await
            }),
        )?
        .await
}
async fn current(
    services: &CoordinatorServices,
    stream: &StreamBinding,
) -> Result<CleanupLease, CoordinatorError> {
    let lookup = stream.clone();
    let head = services
        .io()
        .run(move |journal| journal.head(lookup))
        .await?
        .ok_or(JournalError::NotFound)?;
    services.source().check_current()?;
    Ok(CleanupLease::for_writer(head.lease().clone()))
}
async fn flush(
    services: &CoordinatorServices,
    owner: &CoordinatorPublication,
    stream: &StreamBinding,
    mut target: Option<SnapshotCandidate>,
) -> Result<Option<ResidentSnapshot>, CoordinatorError> {
    for _ in 0..owner.limits.records() {
        services.source().check_current()?;
        if let Some(candidate) = &target
            && candidate.stored().is_some()
        {
            let purged = owner.transport.purge(candidate.clone()).await?;
            // Observe the current writer after await, preserving original signed
            // context while admitting no grant/tool/model operation.
            services.source().check_current()?;
            let lease = current(services, stream).await?;
            let snapshots = owner.snapshots.clone();
            let archive = services
                .io()
                .run(move |_| snapshots.activate(lease, purged))
                .await?;
            services.source().check_current()?;
            return Ok(Some(archive));
        }
        let lookup = stream.clone();
        let ledger = owner.deliveries.clone();
        let after = services.io().run(move |_| ledger.delivered(lookup)).await?;
        services.source().check_current()?;
        let lookup = stream.clone();
        let reader = owner.reader.clone();
        let work = owner.work.clone();
        let limits = owner.limits;
        let publication = services
            .io()
            .run(move |_| reader.next(lookup, after, limits, work))
            .await?;
        services.source().check_current()?;
        let Some(publication) = publication else {
            return if target.is_some() {
                Err(JournalError::Conflict.into())
            } else {
                Ok(None)
            };
        };
        let snapshot = publication.anchor().kind() == HeadAnchorKind::Snapshot;
        let receipt = owner.transport.publish(publication).await?;
        services.source().check_current()?;
        let lease = current(services, stream).await?;
        let ledger = owner.deliveries.clone();
        services
            .io()
            .run(move |_| ledger.record_delivery(lease, receipt))
            .await?;
        services.source().check_current()?;
        if snapshot {
            let snapshots = owner.snapshots.clone();
            let lookup = stream.clone();
            target = services
                .io()
                .run(move |_| snapshots.pending(lookup))
                .await?;
            services.source().check_current()?;
            if target.is_none() {
                return Err(JournalError::Corrupt.into());
            }
        }
    }
    Err(JournalError::Capacity.into())
}
