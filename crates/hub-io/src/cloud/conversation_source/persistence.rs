//! Encrypted last-page persistence under the original physical publication fence.
use super::{
    MAX_WIRE_BYTES, NativeSource, SourceCore,
    cache::{self, CacheFile, CacheMarker, CachedPublication},
    head_cursor,
    heads::FreshHead,
    storage::OutboxFile,
    work::ConversationWorkers,
};
use futures::future::BoxFuture;
use nocterm_cloud_model::projection::{ConversationProjection, ProjectionBinding, ProjectionError};
use nocterm_core::{
    allocation::{Allocated, AllocationLease},
    paths::ensure_private_dir,
};
use nocterm_hub_model::cloud::{ChatCoordinate, RunnerCoordinate, relay::MAX_RELAY_BINDING_BYTES};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
const CACHE_MAX: usize = MAX_WIRE_BYTES
    + nocterm_cloud_model::projection::MAX_REPLAY_EVENTS * (MAX_RELAY_BINDING_BYTES + 64)
    + 4096;
#[derive(Clone)]
pub(super) struct Files(Arc<Drivers>);
pub(super) struct Drivers {
    outbox: Mutex<OutboxFile>,
    cache: Mutex<CacheFile>,
    _allocation: AllocationLease,
}
impl Files {
    pub(super) fn outbox(&self) -> &Mutex<OutboxFile> {
        &self.0.outbox
    }
    fn cache(&self) -> &Mutex<CacheFile> {
        &self.0.cache
    }
}
pub(super) async fn open(
    native: Arc<NativeSource>,
    workers: Arc<ConversationWorkers>,
    storage: Arc<super::drivers::ConversationStorage>,
    binding: &ProjectionBinding,
    head: &FreshHead,
    work: AllocationLease,
) -> Result<Files, ProjectionError> {
    let binding = binding.clone();
    let marker = CacheMarker {
        writer: binding.writer(),
        known: head_cursor(&head.head),
        control: head.head.facts().control,
        deleted: head.head.facts().deleted,
        catalog_revision: head.revision,
        catalog_root: head.root,
        delivered: 0,
    };
    let current = native.clone();
    let output = workers
        .blocking(work, false, move || {
            native.check()?;
            let root = namespace_root(storage.root().to_path_buf(), &native);
            let files = storage.open(root)?;
            let cache = files
                .cache()
                .lock()
                .map_err(|_| ProjectionError::Unavailable)?;
            if let Some(cell) = cache.read()? {
                let chat = chat(&native)?;
                let old = native
                    .material()
                    .with_key(|key| Ok(cache::open_marker(key, chat, binding.writer(), &cell)))
                    .map_err(|_| ProjectionError::Stale)??;
                require_marker(&old, &marker)?;
            }
            native.check()?;
            drop(cache);
            Ok(files)
        })?
        .await
        .map_err(|_| ProjectionError::Unavailable)?;
    current.check()?;
    Ok(output.result()?.clone())
}
fn namespace_root(root: PathBuf, native: &NativeSource) -> PathBuf {
    let request = native.material().request();
    let stream = &request.stream;
    let mut hash = Sha256::new();
    hash.update(b"nocterm-native-cloud-view-source-v1");
    for field in [
        stream.scope().hub().instance().as_str().as_bytes(),
        stream.scope().hub().authority().as_bytes(),
        stream.scope().account().id().as_str().as_bytes(),
        stream.scope().account().key().as_bytes(),
        stream.runner().as_bytes(),
        stream.chat().as_bytes(),
        native.cloud().subject().device().as_str().as_bytes(),
    ] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field);
    }
    hash.update(stream.runner_epoch().get().to_be_bytes());
    hash.update(request.epoch.get().to_be_bytes());
    let suffix = hex::encode(hash.finalize());
    root.join(suffix)
}
fn chat(native: &NativeSource) -> Result<ChatCoordinate, ProjectionError> {
    let request = native.material().request();
    ChatCoordinate::new(
        native.cloud().namespace().clone(),
        RunnerCoordinate {
            id: *request.stream.runner().as_bytes(),
            key_epoch: request.stream.runner_epoch().get(),
        },
        *request.stream.chat().as_bytes(),
        request.epoch.get(),
    )
    .map_err(|_| ProjectionError::WrongSource)
}
fn require_marker(old: &CacheMarker, new: &CacheMarker) -> Result<(), ProjectionError> {
    if new.writer < old.writer
        || new.control < old.control
        || (old.deleted && !new.deleted)
        || new.catalog_revision < old.catalog_revision
        || (new.catalog_revision == old.catalog_revision && new.catalog_root != old.catalog_root)
        || new.known.sequence() < old.known.sequence()
        || (new.known.sequence() == old.known.sequence() && new.known.head() != old.known.head())
    {
        return Err(ProjectionError::Rollback);
    }
    Ok(())
}
/// Persist fully verified ciphertexts BEFORE advancing the scan/ACK watermark.
/// The caller owns this whole page and its reservation throughout physical I/O.
pub(super) fn store_page(
    core: &SourceCore,
    marker: &CacheMarker,
    records: &[CachedPublication],
) -> Result<(), ProjectionError> {
    core.check()?;
    let file = core
        .files
        .cache()
        .lock()
        .map_err(|_| ProjectionError::Unavailable)?;
    let chat = core.chat()?;
    if let Some(cell) = file.read()? {
        let old = core
            .native
            .material()
            .with_key(|key| {
                Ok(cache::open_marker(
                    key,
                    chat.clone(),
                    core.binding.writer(),
                    &cell,
                ))
            })
            .map_err(|_| ProjectionError::Stale)??;
        require_marker(&old, marker)?;
    }
    let sealed = core
        .native
        .material()
        .with_key(|key| Ok(cache::seal(key, chat, marker, records)))
        .map_err(|_| ProjectionError::Stale)??;
    let staged = file.stage(&sealed, core.binding.writer())?;
    core.publish(|| staged.publish().map_err(|_| ProjectionError::Persistence))??;
    let durable = file.read()?.ok_or(ProjectionError::Persistence)?;
    // read is bounded/authenticated above; directory fsync is part of durability,
    // unlike AtomicPublication's explicitly best-effort convenience sync.
    let opened = nocterm_core::fs::open_private_regular(file.path())
        .map_err(|_| ProjectionError::Persistence)?;
    file.confirm(&opened)?;
    if durable.writer != core.binding.writer().get() {
        return Err(ProjectionError::Persistence);
    }
    core.check()
}
pub(super) fn confirm(
    core: Arc<SourceCore>,
    projection: Arc<Allocated<ConversationProjection>>,
    work: AllocationLease,
) -> BoxFuture<'static, Result<(), ProjectionError>> {
    Box::pin(async move {
        core.require_work(&work, 64 * 1024)?;
        if projection.value().binding() != &core.binding {
            return Err(ProjectionError::WrongSource);
        }
        let cursor = projection.value().cursor();
        let current = core.native.clone();
        let workers = core.workers.clone();
        let output = workers
            .blocking(work, false, move || {
                core.check()?;
                let progress = core
                    .progress
                    .lock()
                    .map_err(|_| ProjectionError::Unavailable)?;
                progress.routes.delivery(cursor)?;
                if cursor.sequence() > progress.routes.high().sequence()
                    || projection.value().control() > progress.head.head.facts().control
                {
                    return Err(ProjectionError::Rollback);
                }
                // next already durably published the raw verified page and marker.
                // No native projection JSON or invented full snapshot is written.
                drop(progress);
                core.publish(|| ())
            })?
            .await
            .map_err(|_| ProjectionError::Unavailable)?;
        current.check()?;
        output.result().copied()
    })
}

impl Files {
    pub(super) fn create(
        root: &std::path::Path,
        allocation: AllocationLease,
    ) -> Result<Self, ProjectionError> {
        ensure_private_dir(root).map_err(|_| ProjectionError::Persistence)?;
        let outbox = OutboxFile::open(
            root.join("outbox"),
            nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES + 4096,
        )?;
        let cache_root = root.join("replay");
        ensure_private_dir(&cache_root).map_err(|_| ProjectionError::Persistence)?;
        let cache = CacheFile::new(cache_root, CACHE_MAX)?;
        Ok(Self(Arc::new(Drivers {
            outbox: Mutex::new(outbox),
            cache: Mutex::new(cache),
            _allocation: allocation,
        })))
    }
    pub(super) fn weak(&self) -> std::sync::Weak<Drivers> {
        Arc::downgrade(&self.0)
    }
    pub(super) fn upgrade(driver: &std::sync::Weak<Drivers>) -> Option<Self> {
        driver.upgrade().map(Self)
    }
}
