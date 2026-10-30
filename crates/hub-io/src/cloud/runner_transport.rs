//! Immutable outbox delivery; signed event order and relay storage cursors differ.
use super::{
    RunnerHubPeer,
    runner_peer::{RunnerHubError, sdk_error},
};
use nocterm_cloud_model::{
    authority::{RunBinding, StreamBinding},
    identity::{ChatKeyEpoch, EventId, Fingerprint},
    journal::{
        DeliveryCursor, HeadAnchor, HeadAnchorKind, HeadRecordPosition, JournalCatalogBinding,
        JournalError, JournalFuture, OutboxPublicationTransport, OutboxStored,
        PublicationAllocation, ResidentPublication, SnapshotCandidate, SnapshotPurged,
    },
};
use nocterm_core::SharedCause;
use nocterm_hub_client::cloud::{CloudError, CloudSelector, CloudUpload};
use nocterm_hub_model::cloud::{
    ChatCoordinate, RecordContext, RecordPosition, RunCoordinate,
    relay::{PublicationMetadata, RelayBinding},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

impl OutboxPublicationTransport for RunnerHubPeer {
    fn publish(&self, publication: ResidentPublication) -> JournalFuture<OutboxStored> {
        let connection = self.connection().clone();
        let horizons = self.horizons().clone();
        Box::pin(async move {
            if !connection.binding().contains(publication.stream()) {
                return Err(JournalError::Fenced);
            }
            let runner = connection.runner().map_err(native_error)?;
            // This bound is supplied by the SDK that owns the queue and headers.
            // The same actual reservation follows both upload and RPC task.
            let allocation = connection.work().try_reserve(PublicationAllocation::new(
                CloudUpload::buffer_work_bytes(publication.sealed().len())
                    .map_err(transport_error)?,
                1,
            )?)?;
            let metadata = metadata(&publication)?;
            connection.runner().map_err(native_error)?;
            let residency = Arc::new((publication.clone(), allocation));
            // A hub with reclaim may drop the receipt after this horizon, which stays the
            // same for every attempt at this publication; any other hub gets the previous
            // request unchanged.
            let horizon = horizons.horizon(
                *publication.delivery_id().as_bytes(),
                super::runner_reclaim::now_ms().map_err(native_error)?,
                runner.supports_receipt_reclaim(),
            );
            let mut upload = if horizon == 0 {
                runner.begin_publication_with_residency(metadata, residency.clone())
            } else {
                runner.begin_publication_with_retry_until_and_residency(
                    metadata,
                    horizon,
                    residency.clone(),
                )
            }
            .map_err(transport_error)?;
            let frame_bytes =
                usize::try_from(upload.max_frame_bytes()).map_err(|_| JournalError::Capacity)?;
            if frame_bytes == 0 {
                return Err(JournalError::Capacity);
            }
            for bytes in publication.sealed().as_bytes().chunks(frame_bytes) {
                connection.runner().map_err(native_error)?;
                let response = upload.write(bytes).await;
                connection.runner().map_err(native_error)?;
                response.map_err(transport_error)?;
            }
            connection.runner().map_err(native_error)?;
            let response = upload.finish().await;
            connection.runner().map_err(native_error)?;
            let receipt = response.map_err(transport_error)?;
            if receipt.publication_id.as_slice() != publication.delivery_id().as_bytes()
                || receipt.ciphertext_sha256.as_slice()
                    != publication.anchor().ciphertext().as_bytes()
                || receipt.cursor == 0
            {
                return Err(JournalError::Corrupt);
            }
            horizons.settle(publication.delivery_id().as_bytes());
            Ok(OutboxStored::new(
                publication.stream().clone(),
                publication.delivery_id(),
                publication.anchor().ciphertext(),
                DeliveryCursor::new(receipt.cursor)?,
            ))
        })
    }

    fn purge(&self, candidate: SnapshotCandidate) -> JournalFuture<SnapshotPurged> {
        let connection = self.connection().clone();
        Box::pin(async move {
            if !connection.binding().contains(candidate.stream()) {
                return Err(JournalError::Fenced);
            }
            connection.runner().map_err(native_error)?;
            // Unary retention still owns bounded routing/proof/receipt scratch;
            // the SDK's minimum publication working bound covers those buffers.
            let work = connection.work().try_reserve(PublicationAllocation::new(
                CloudUpload::buffer_work_bytes(1).map_err(transport_error)?,
                1,
            )?)?;
            let stored = purge_receipt(&candidate)?;
            let through = stored
                .cursor()
                .get()
                .checked_sub(1)
                .ok_or(JournalError::Corrupt)?;
            let selector = CloudSelector::Chat {
                runner: super::metadata_crypto::runner(connection.binding()),
                chat: *candidate.stream().chat().as_bytes(),
            };
            let runner = connection.runner().map_err(native_error)?;
            let residency = Arc::new(work);
            let response = runner
                .purge_with_residency(
                    &selector,
                    through,
                    *candidate.delivery_id().as_bytes(),
                    residency.clone(),
                )
                .await;
            connection.runner().map_err(native_error)?;
            if response.map_err(transport_error)? != through {
                return Err(JournalError::Corrupt);
            }
            SnapshotPurged::new(stored, through)
        })
    }
}

fn metadata(publication: &ResidentPublication) -> Result<PublicationMetadata, JournalError> {
    let digest = Fingerprint::from_bytes(Sha256::digest(publication.sealed().as_bytes()).into());
    if digest != publication.anchor().ciphertext() {
        return Err(JournalError::Corrupt);
    }
    let context = record_context(
        publication.stream(),
        publication.key_epoch(),
        publication.anchor(),
    )?;
    let binding = match publication.anchor().kind() {
        HeadAnchorKind::Event => RelayBinding::event(context),
        HeadAnchorKind::Snapshot => RelayBinding::snapshot(context),
    };
    PublicationMetadata::new(
        *publication.delivery_id().as_bytes(),
        binding,
        u32::try_from(publication.sealed().len()).map_err(|_| JournalError::Capacity)?,
        *digest.as_bytes(),
    )
    .map_err(storage_error)
}

/// One adapter-private native-to-public coordinate map for encoder and transport.
pub(super) fn record_context(
    stream: &StreamBinding,
    epoch: ChatKeyEpoch,
    anchor: &HeadAnchor,
) -> Result<RecordContext, JournalError> {
    record_context_parts(stream, epoch, anchor.position(), anchor.id(), anchor.run())
}

pub(super) fn record_context_parts(
    stream: &StreamBinding,
    epoch: ChatKeyEpoch,
    position: HeadRecordPosition,
    id: EventId,
    run: Option<&RunBinding>,
) -> Result<RecordContext, JournalError> {
    if run.is_some_and(|run| run.stream() != *stream) {
        return Err(JournalError::Fenced);
    }
    let binding = JournalCatalogBinding::new(
        stream.scope().clone(),
        stream.runner(),
        stream.runner_epoch(),
    );
    let chat = ChatCoordinate::new(
        super::metadata_crypto::namespace(&binding)?,
        super::metadata_crypto::runner(&binding),
        *stream.chat().as_bytes(),
        epoch.get(),
    )
    .map_err(storage_error)?;
    RecordContext::new(
        chat,
        RecordPosition {
            writer_epoch: position.writer().get(),
            sequence: position.event().get(),
            journal_sequence: position.journal().get(),
            revision: position.revision().get(),
        },
        *id.as_bytes(),
        run.map(|run| RunCoordinate {
            id: *run.run().as_bytes(),
            expected_epoch: run.epoch().get(),
        }),
    )
    .map_err(storage_error)
}

fn purge_receipt(candidate: &SnapshotCandidate) -> Result<OutboxStored, JournalError> {
    if candidate.anchor().kind() != HeadAnchorKind::Snapshot {
        return Err(JournalError::Conflict);
    }
    let stored = candidate.stored().ok_or(JournalError::Conflict)?;
    if stored.stream() != candidate.stream()
        || stored.delivery_id() != candidate.delivery_id()
        || stored.ciphertext() != &candidate.anchor().ciphertext()
    {
        return Err(JournalError::Conflict);
    }
    record_context(candidate.stream(), candidate.material(), candidate.anchor())?;
    Ok(stored.clone())
}
fn native_error(error: RunnerHubError) -> JournalError {
    match error {
        RunnerHubError::Binding
        | RunnerHubError::Unavailable
        | RunnerHubError::Identity(_)
        | RunnerHubError::SourceChanged(_) => JournalError::Fenced,
        error => storage_error(error),
    }
}
fn transport_error(error: CloudError) -> JournalError {
    native_error(sdk_error(error))
}
fn storage_error(error: impl std::error::Error + Send + Sync + 'static) -> JournalError {
    JournalError::Storage(SharedCause::new(error))
}

#[cfg(test)]
mod tests;
