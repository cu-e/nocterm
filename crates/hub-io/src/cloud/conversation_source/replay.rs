//! Verified actual relay positions, independent from signed logical event cursors.
use super::{
    MAX_WIRE_BYTES, SourceCore,
    cache::{CacheMarker, CachedPublication},
    head_cursor, heads, observation_error, persistence, requirements, sdk_error,
};
use nocterm_cloud_model::{
    journal::HeadAnchor,
    projection::{
        MAX_REPLAY_BYTES, MAX_REPLAY_EVENTS, ProjectionBatch, ProjectionCursor, ProjectionError,
        ProjectionEvent, RecordKind as Fault,
    },
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_client::cloud::{CloudDeliveryOrigin, CloudReplayItem, CloudSelector};
use nocterm_hub_model::cloud::{
    RecordKind,
    head::HeadRecordAnchor,
    relay::{PublicationMetadata, RelayKind},
};
use std::sync::Arc;

pub(super) enum NativePoll {
    Page(Box<ProjectionBatch>),
    Readmission,
}
struct Page {
    events: Vec<ProjectionEvent>,
    records: Vec<CachedPublication>,
    cursor: ProjectionCursor,
    delivery: u64,
    retained: usize,
    wire: usize,
}
impl Page {
    fn new(cursor: ProjectionCursor, delivery: u64) -> Self {
        Self {
            events: Vec::with_capacity(MAX_REPLAY_EVENTS),
            records: Vec::with_capacity(MAX_REPLAY_EVENTS),
            cursor,
            delivery,
            retained: 1024,
            wire: 0,
        }
    }
    fn push(
        &mut self,
        core: &SourceCore,
        metadata: PublicationMetadata,
        delivery: u64,
        ciphertext: Vec<u8>,
        work: &AllocationLease,
    ) -> Result<(), ProjectionError> {
        if self.events.len() == MAX_REPLAY_EVENTS || delivery <= self.delivery {
            return Err(ProjectionError::Discontinuous);
        }
        let anchor = anchor(core, &metadata)?;
        let remaining = MAX_REPLAY_BYTES
            .checked_sub(self.retained)
            .ok_or(ProjectionError::Capacity)?;
        let scratch = crate::cloud::AccountObservationDecoder::work_bytes(ciphertext.len(), false)
            .map_err(observation_error)?;
        // Restriction of ONE real grant, never a new quota. Whole-page work also
        // covers its existing events and the temporary admitted clone generation.
        let restricted = AllocationLease::new(
            scratch
                .checked_add(remaining)
                .ok_or(ProjectionError::Capacity)?,
            1,
            work.clone(),
        );
        let value = core
            .decoder
            .event(&anchor, self.cursor, &ciphertext, restricted)
            .map_err(observation_error)?;
        let size = value.value().resident_bytes()?;
        if size > remaining {
            return Err(ProjectionError::Capacity);
        }
        self.retained = self
            .retained
            .checked_add(size)
            .ok_or(ProjectionError::Capacity)?;
        self.cursor = value.value().after()?;
        self.delivery = delivery;
        self.events.push(value.value().clone());
        self.records.push(CachedPublication {
            metadata,
            delivery,
            ciphertext,
        });
        Ok(())
    }
}
pub(super) async fn page(
    core: Arc<SourceCore>,
    after: ProjectionCursor,
    work: AllocationLease,
) -> Result<NativePoll, ProjectionError> {
    core.require_work(&work, heads::work_bytes())?;
    let probes = heads::ProbeWork::new(work.clone())?;
    let mut delivery_deadline = None;
    loop {
        core.network(core.refresh(&probes)).await?;
        let (delivery, known) = {
            let state = core
                .progress
                .lock()
                .map_err(|_| ProjectionError::Unavailable)?;
            (state.routes.delivery(after)?, head_cursor(&state.head.head))
        };
        if known.sequence() < after.sequence()
            || (known.sequence() == after.sequence() && known.head() != after.head())
        {
            return Err(ProjectionError::Rollback);
        }
        if known == after {
            core.idle().await?;
            continue;
        }
        if work.bytes() < requirements::replay(MAX_WIRE_BYTES)? {
            return Ok(NativePoll::Readmission);
        }
        let deadline = *delivery_deadline.get_or_insert_with(|| {
            tokio::time::Instant::now() + std::time::Duration::from_secs(10)
        });
        if tokio::time::Instant::now() >= deadline {
            return Err(ProjectionError::Unavailable);
        }
        let mut page = Page::new(after, delivery);
        tokio::time::timeout_at(deadline, core.network(read(&core, &mut page, &work)))
            .await
            .map_err(|_| ProjectionError::Unavailable)??;
        if page.events.is_empty() {
            core.idle().await?;
            continue;
        }
        core.network(core.refresh(&probes)).await?;
        let mut marker = core.marker()?;
        if marker.known.sequence() < page.cursor.sequence()
            || (marker.known.sequence() == page.cursor.sequence()
                && marker.known.head() != page.cursor.head())
        {
            return Err(ProjectionError::Rollback);
        }
        marker.delivered = page.delivery;
        let result = persist(core.clone(), page, after, marker, work.clone()).await?;
        return Ok(NativePoll::Page(Box::new(result)));
    }
}
async fn read(
    core: &SourceCore,
    page: &mut Page,
    work: &AllocationLease,
) -> Result<(), ProjectionError> {
    let selector = CloudSelector::Chat {
        runner: nocterm_hub_model::cloud::RunnerCoordinate {
            id: *core.binding.stream().runner().as_bytes(),
            key_epoch: core.binding.stream().runner_epoch().get(),
        },
        chat: *core.binding.stream().chat().as_bytes(),
    };
    let mut stream = core
        .native
        .cloud()
        .replay_with_residency(
            selector,
            page.delivery,
            MAX_REPLAY_EVENTS as u32,
            MAX_WIRE_BYTES as u32,
            Arc::new(work.clone()),
        )
        .await
        .map_err(sdk_error)?;
    loop {
        core.check()?;
        match stream.next_record().await.map_err(sdk_error)? {
            CloudReplayItem::Record(mut record) => {
                require_origin(
                    core.binding.stream(),
                    &core.native.material().request().pins.proof,
                    record.origin(),
                )?;
                let metadata = record.metadata().clone();
                let delivery = record.cursor();
                let length = usize::try_from(metadata.total_bytes())
                    .map_err(|_| ProjectionError::Capacity)?;
                if length > MAX_WIRE_BYTES.saturating_sub(page.wire) {
                    return Err(ProjectionError::Capacity);
                }
                let mut ciphertext = Vec::with_capacity(length);
                while let Some(bytes) = record.next_chunk().await.map_err(sdk_error)? {
                    core.check()?;
                    if bytes.len() > length.saturating_sub(ciphertext.len()) {
                        return Err(ProjectionError::InvalidRecord(Fault::Replay));
                    }
                    ciphertext.extend_from_slice(&bytes);
                }
                if ciphertext.len() != length {
                    return Err(ProjectionError::InvalidRecord(Fault::Replay));
                }
                page.wire = page
                    .wire
                    .checked_add(length)
                    .ok_or(ProjectionError::Capacity)?;
                page.push(core, metadata, delivery, ciphertext, work)?;
            }
            CloudReplayItem::End { .. } => return Ok(()),
            CloudReplayItem::Gap { .. } => return Err(ProjectionError::Unsupported),
        }
    }
}
pub(super) fn require_origin(
    stream: &nocterm_cloud_model::authority::StreamBinding,
    proof: &[u8; 32],
    origin: &CloudDeliveryOrigin,
) -> Result<(), ProjectionError> {
    match origin {
        CloudDeliveryOrigin::Runner {
            account,
            runner,
            key,
        } if account.as_str() == stream.scope().account().id().as_str()
            && runner == stream.runner().as_bytes()
            && key == proof =>
        {
            Ok(())
        }
        _ => Err(ProjectionError::WrongSource),
    }
}
fn anchor(
    core: &SourceCore,
    metadata: &PublicationMetadata,
) -> Result<HeadAnchor, ProjectionError> {
    if metadata.binding().kind() != RelayKind::Event {
        return Err(ProjectionError::Unsupported);
    }
    let context = metadata
        .binding()
        .record_context()
        .ok_or(ProjectionError::InvalidRecord(Fault::Replay))?;
    if context.chat() != &core.chat()? {
        return Err(ProjectionError::WrongSource);
    }
    heads::anchor(
        &core.native,
        &HeadRecordAnchor {
            kind: RecordKind::Event,
            position: context.position(),
            id: context.id(),
            run: context.run(),
            ciphertext_digest: metadata.digest(),
        },
    )
}
async fn persist(
    core: Arc<SourceCore>,
    page: Page,
    after: ProjectionCursor,
    marker: CacheMarker,
    work: AllocationLease,
) -> Result<ProjectionBatch, ProjectionError> {
    let workers = core.workers.clone();
    let native = core.native.clone();
    let output = workers
        .blocking(work, false, move || {
            core.check()?;
            core.progress
                .lock()
                .map_err(|_| ProjectionError::Unavailable)?
                .routes
                .delivery(after)?;
            persistence::store_page(&core, &marker, &page.records)?;
            let batch = ProjectionBatch::new(core.binding.clone(), page.events, marker.known)?;
            retire_admitted(&core, &batch)?;
            let mut state = core
                .progress
                .lock()
                .map_err(|_| ProjectionError::Unavailable)?;
            state.routes.offer(after, page.cursor, page.delivery)?;
            Ok(batch)
        })?
        .await
        .map_err(|_| ProjectionError::Unavailable)?;
    native.check()?;
    // The two pre-admitted DTO generations cover the original physical result
    // and this runtime publication clone; no unguarded extraction is used.
    Ok(output.result()?.clone())
}

fn retire_admitted(core: &SourceCore, batch: &ProjectionBatch) -> Result<(), ProjectionError> {
    use nocterm_cloud_model::coordinator::CoordinatorEvent;
    for envelope in batch.events() {
        let command = match &envelope.event {
            CoordinatorEvent::CommandAccepted { command, .. }
            | CoordinatorEvent::PromptAccepted { command, .. } => Some(command),
            _ => None,
        };
        if let Some(command) = command
            && let Err(error) = super::commands::retire(core, command.id())
        {
            core.progress
                .lock()
                .map_err(|_| ProjectionError::Unavailable)?
                .maintenance = Some(error);
        }
    }
    Ok(())
}
