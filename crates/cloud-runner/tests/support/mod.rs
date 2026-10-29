#![allow(dead_code)]
mod metadata;
pub(crate) type RootStateSnapshot = metadata::RootStateSnapshot;
use nocterm_cloud_model::{
    authority::*, engine::EngineCheckpoint, identity::*, journal::*, payload::OpaqueBytes,
};
use nocterm_cloud_runner::{DurableRunJournal, JournalLimits};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

/// Reversible test codec with a context checksum, NOT a cryptography proof.
/// Production has no such implementation and always requires injected AEAD.
#[derive(Default)]
pub(crate) struct TestCipher {
    fail: AtomicBool,
    retired: AtomicBool,
    catalog_opens: AtomicUsize,
    head_opens: AtomicUsize,
    outbox_opens: AtomicUsize,
    open_barrier: Mutex<Option<OpenPause>>,
    seal_barrier: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}
struct OpenPause {
    class: JournalStorageClass,
    entered: mpsc::Sender<()>,
    resumed: mpsc::Receiver<()>,
}
impl TestCipher {
    pub(crate) fn metadata_opens(&self) -> (usize, usize) {
        (
            self.catalog_opens.load(Ordering::SeqCst),
            self.head_opens.load(Ordering::SeqCst),
        )
    }
    pub(crate) fn retire_content_key(&self) {
        self.retired.store(true, Ordering::SeqCst);
    }
    pub(crate) fn pause_next_seal(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, observed) = mpsc::channel();
        let (release, resumed) = mpsc::channel();
        *self.seal_barrier.lock().unwrap() = Some((entered, resumed));
        (observed, release)
    }
    pub(crate) fn outbox_opens(&self) -> usize {
        self.outbox_opens.load(Ordering::SeqCst)
    }
    pub(crate) fn pause_next_open(
        &self,
        class: JournalStorageClass,
    ) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, observed) = mpsc::channel();
        let (release, resumed) = mpsc::channel();
        *self.open_barrier.lock().unwrap() = Some(OpenPause {
            class,
            entered,
            resumed,
        });
        (observed, release)
    }
    pub(crate) fn fail_sealing(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }
}
impl JournalCipher for TestCipher {
    fn chat_key_epoch(&self, _stream: &StreamBinding) -> Result<ChatKeyEpoch, JournalError> {
        if self.retired.load(Ordering::SeqCst) {
            return Err(JournalError::KeyUnavailable);
        }
        ChatKeyEpoch::new(7).map_err(JournalError::from)
    }
    fn seal(&self, context: &JournalCipherContext, plain: &[u8]) -> Result<Vec<u8>, JournalError> {
        if self.retired.load(Ordering::SeqCst) {
            return Err(JournalError::KeyUnavailable);
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(JournalError::Corrupt);
        }
        let barrier = self.seal_barrier.lock().unwrap().take();
        if let Some((entered, resumed)) = barrier {
            entered.send(()).unwrap();
            resumed.recv().unwrap();
        }
        let mut bytes = checksum(context, plain).to_vec();
        bytes.extend(plain.iter().map(|byte| byte ^ 0xa5));
        Ok(bytes)
    }
    fn open(&self, context: &JournalCipherContext, sealed: &[u8]) -> Result<Vec<u8>, JournalError> {
        if context.class() == JournalStorageClass::Outbox {
            self.outbox_opens.fetch_add(1, Ordering::SeqCst);
        }
        let barrier = {
            let mut barrier = self.open_barrier.lock().unwrap();
            if barrier
                .as_ref()
                .is_some_and(|pause| pause.class == context.class())
            {
                barrier.take()
            } else {
                None
            }
        };
        if let Some(pause) = barrier {
            pause.entered.send(()).unwrap();
            pause.resumed.recv().unwrap();
        }
        if self.retired.load(Ordering::SeqCst) {
            return Err(JournalError::KeyUnavailable);
        }
        if sealed.len() < 32 {
            return Err(JournalError::Corrupt);
        }
        let bytes: Vec<u8> = sealed[32..].iter().map(|byte| byte ^ 0xa5).collect();
        if checksum(context, &bytes) != sealed[..32] {
            return Err(JournalError::Corrupt);
        }
        Ok(bytes)
    }
}
impl JournalCatalogCipher for TestCipher {
    fn seal(&self, context: &JournalCatalogContext, plain: &[u8]) -> Result<Vec<u8>, JournalError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(JournalError::Corrupt);
        }
        let mut sealed = catalog_checksum(context, plain).to_vec();
        sealed.extend(plain.iter().map(|byte| byte ^ 0xa5));
        Ok(sealed)
    }
    fn open(
        &self,
        context: &JournalCatalogContext,
        sealed: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        self.catalog_opens.fetch_add(1, Ordering::SeqCst);
        if sealed.len() < 32 || sealed.len() > MAX_JOURNAL_CATALOG_CIPHERTEXT {
            return Err(JournalError::Corrupt);
        }
        let plain: Vec<u8> = sealed[32..].iter().map(|byte| byte ^ 0xa5).collect();
        if catalog_checksum(context, &plain) != sealed[..32] {
            return Err(JournalError::Corrupt);
        }
        Ok(plain)
    }
}
impl JournalChatHeadCipher for TestCipher {
    fn seal(
        &self,
        context: &JournalChatHeadContext,
        plain: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        if plain.is_empty() || plain.len() > MAX_JOURNAL_CHAT_HEAD_PLAINTEXT {
            return Err(JournalError::Capacity);
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(JournalError::Corrupt);
        }
        let mut sealed = head_checksum(context, plain).to_vec();
        sealed.extend(plain.iter().map(|byte| byte ^ 0xa5));
        Ok(sealed)
    }
    fn open(
        &self,
        context: &JournalChatHeadContext,
        sealed: &[u8],
    ) -> Result<Vec<u8>, JournalError> {
        self.head_opens.fetch_add(1, Ordering::SeqCst);
        if sealed.len() <= 32 || sealed.len() > MAX_JOURNAL_CHAT_HEAD_CIPHERTEXT {
            return Err(JournalError::Corrupt);
        }
        let plain: Vec<u8> = sealed[32..].iter().map(|byte| byte ^ 0xa5).collect();
        if head_checksum(context, &plain) != sealed[..32] {
            return Err(JournalError::Corrupt);
        }
        Ok(plain)
    }
}
fn head_checksum(context: &JournalChatHeadContext, plain: &[u8]) -> [u8; 32] {
    let stream = context.stream();
    let mut hash = Sha256::new();
    hash.update(b"TEST-HEAD-NOT-CRYPTO");
    for bytes in [
        stream.scope().hub().instance().as_str().as_bytes(),
        stream.scope().hub().authority().as_bytes(),
        stream.scope().account().id().as_str().as_bytes(),
        stream.scope().account().key().as_bytes(),
        stream.runner().as_bytes(),
        stream.chat().as_bytes(),
        plain,
    ] {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hash.update(stream.runner_epoch().get().to_be_bytes());
    hash.update([u8::from(context.writer().is_some())]);
    if let Some(writer) = context.writer() {
        hash.update(writer.get().to_be_bytes());
    }
    hash.update(context.revision().to_be_bytes());
    hash.finalize().into()
}
fn catalog_checksum(context: &JournalCatalogContext, plain: &[u8]) -> [u8; 32] {
    let binding = context.binding();
    let scope = binding.scope();
    let mut hash = Sha256::new();
    hash.update(b"TEST-CATALOG-NOT-CRYPTO");
    for bytes in [
        scope.hub().instance().as_str().as_bytes(),
        scope.hub().authority().as_bytes(),
        scope.account().id().as_str().as_bytes(),
        scope.account().key().as_bytes(),
        binding.runner().as_bytes(),
        plain,
    ] {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hash.update(binding.epoch().get().to_be_bytes());
    hash.update(context.revision().to_be_bytes());
    hash.finalize().into()
}
fn checksum(context: &JournalCipherContext, plain: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    let stream = context.stream();
    for value in [
        stream.scope().hub().instance().as_str().as_bytes(),
        stream.scope().hub().authority().as_bytes(),
        stream.scope().account().id().as_str().as_bytes(),
        stream.scope().account().key().as_bytes(),
        stream.runner().as_bytes(),
        stream.chat().as_bytes(),
        context.id(),
        plain,
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    hash.update(stream.runner_epoch().get().to_be_bytes());
    let class = match context.class() {
        JournalStorageClass::Header => 1,
        JournalStorageClass::Record => 2,
        JournalStorageClass::CommandReceipt => 3,
        JournalStorageClass::Checkpoint => 4,
        JournalStorageClass::Snapshot => 5,
        JournalStorageClass::Outbox => 6,
        JournalStorageClass::BackgroundCommand => 7,
        JournalStorageClass::Effect => 8,
        JournalStorageClass::RunHead => 9,
    };
    hash.update([class]);
    hash.update(context.sequence().get().to_be_bytes());
    hash.update(context.revision().get().to_be_bytes());
    hash.update(context.writer().get().to_be_bytes());
    hash.finalize().into()
}
pub(crate) fn fp(byte: u8) -> Fingerprint {
    Fingerprint::from_bytes([byte; 32])
}
pub(crate) fn stream() -> StreamBinding {
    StreamBinding::new(
        AccountScope::new(
            HubIdentity::new(NamespaceId::new("hub").unwrap(), fp(1)),
            AccountIdentity::new(NamespaceId::new("account").unwrap(), fp(2)),
        ),
        RunnerId::from_bytes([3; 16]),
        RunnerKeyEpoch::new(1).unwrap(),
        CloudChatId::from_bytes([4; 16]),
    )
}
pub(crate) fn run() -> RunBinding {
    let stream = stream();
    RunBinding::new(
        stream.scope().clone(),
        stream.runner(),
        stream.runner_epoch(),
        stream.chat(),
        RunId::from_bytes([5; 16]),
        RunEpoch::new(1).unwrap(),
    )
}
pub(crate) fn bytes(value: &[u8]) -> OpaqueBytes {
    OpaqueBytes::new(value.to_vec()).unwrap()
}
pub(crate) fn limits() -> JournalReadLimits {
    JournalReadLimits::new(256, 128 * 1024 * 1024).unwrap()
}
pub(crate) fn catalog_binding() -> JournalCatalogBinding {
    let stream = stream();
    JournalCatalogBinding::new(
        stream.scope().clone(),
        stream.runner(),
        stream.runner_epoch(),
    )
}
/// Test-fixture convenience only: production callers must choose create/open.
pub(crate) fn open_journal(
    root: &std::path::Path,
    cipher: Arc<dyn JournalCipher>,
    limits: JournalLimits,
) -> Result<DurableRunJournal, JournalError> {
    let catalog = Arc::new(TestCipher::default());
    if root.join("cloud-runs.sqlite3").exists() {
        DurableRunJournal::open_existing(
            root,
            catalog_binding(),
            cipher,
            catalog.clone(),
            catalog,
            limits,
        )
    } else {
        DurableRunJournal::create_new(
            root,
            catalog_binding(),
            cipher,
            catalog.clone(),
            catalog,
            limits,
        )
    }
}
pub(crate) fn journal(root: &std::path::Path, cipher: Arc<TestCipher>) -> DurableRunJournal {
    open_journal(root, cipher, JournalLimits::default()).unwrap()
}
pub(crate) fn claim(journal: &DurableRunJournal, previous: Option<WriterEpoch>) -> WriterLease {
    let next = previous.map_or(WriterEpoch::new(1).unwrap(), |epoch| epoch.next().unwrap());
    futures::executor::block_on(
        journal.claim_writer(WriterClaim::new(stream(), previous, next).unwrap()),
    )
    .unwrap()
}
pub(crate) fn acceptance(id: u8, expected: u64) -> CommandAcceptance {
    CommandAcceptance::new(
        CommandId::from_bytes([id; 16]),
        CommandKind::SubmitPrompt,
        fp(id),
        CommandIssuer::new(NamespaceId::new("device").unwrap(), fp(7), fp(8)),
        ChatControlRevision::from_u64(expected),
    )
}
pub(crate) fn admission(
    lease: &WriterLease,
    position: JournalPosition,
    command: CommandAcceptance,
) -> JournalBatch {
    let record = JournalRecord::new(
        RecordContext::Run(run()),
        JournalRecordKind::CommandAccepted {
            id: command.id(),
            kind: command.kind(),
        },
        bytes(b"private prompt plaintext"),
    )
    .unwrap();
    let checkpoint = EngineCheckpoint::new(
        run(),
        NamespaceId::new("test-engine/1").unwrap(),
        position.revision().next().unwrap(),
        bytes(b"private engine checkpoint"),
    )
    .unwrap();
    JournalBatch::new(
        lease.clone(),
        position,
        Some(command.clone()),
        vec![record],
        Some(checkpoint),
        vec![fixture_event(
            lease,
            position.next(1, 1, true).unwrap(),
            EventId::from_bytes(*command.id().as_bytes()),
            Some(run()),
            bytes(b"exact sealed event"),
        )],
    )
    .unwrap()
}
pub(crate) fn commit(journal: &DurableRunJournal, batch: JournalBatch) -> JournalPosition {
    match futures::executor::block_on(journal.commit(batch)).unwrap() {
        CommitOutcome::Committed(receipt) => receipt.position(),
        CommitOutcome::Duplicate(receipt) => receipt.position(),
    }
}

pub(crate) fn private_root() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir().unwrap()
}

pub(crate) fn fixture_event(
    lease: &WriterLease,
    next: JournalPosition,
    id: EventId,
    run: Option<RunBinding>,
    sealed: OpaqueBytes,
) -> ResidentPublication {
    nocterm_cloud_model::testing::fixture_outbox(lease, next, id, run, sealed).unwrap()
}

/// Actual SQLite snapshot owner with explicitly non-cryptographic public fixture.
pub(crate) fn snapshot_publication(
    store: &DurableRunJournal,
) -> nocterm_cloud_runner::DurableSnapshotPublication {
    nocterm_cloud_runner::DurableSnapshotPublication::new(
        store.clone(),
        Arc::new(nocterm_cloud_model::testing::ScriptedSnapshotEncoder),
        Arc::new(nocterm_cloud_model::testing::fixture_snapshot_budget()),
        SnapshotLimits::new(4096, 64 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
    )
}
pub(crate) fn publication_config(
    store: &DurableRunJournal,
    transport: Arc<nocterm_cloud_model::testing::ScriptedPublicationTransport>,
) -> Arc<nocterm_cloud_runner::coordinator::CoordinatorPublication> {
    let publication = Arc::new(snapshot_publication(store));
    Arc::new(
        nocterm_cloud_runner::coordinator::CoordinatorPublication::new(
            publication.clone(),
            publication.clone(),
            publication,
            transport,
            Arc::new(nocterm_cloud_model::testing::fixture_snapshot_budget()),
            limits(),
            16,
        )
        .unwrap(),
    )
}
/// Exercises the actual native stage -> FIFO receipts -> purge -> activation.
/// The shared transport/encoder are test-only descriptions, not public crypto.
pub(crate) fn finish_snapshot(store: &DurableRunJournal, lease: &WriterLease) -> ResidentSnapshot {
    finish_snapshot_with_receipt(store, lease).0
}
pub(crate) fn finish_snapshot_with_receipt(
    store: &DurableRunJournal,
    lease: &WriterLease,
) -> (ResidentSnapshot, SnapshotPurged) {
    use nocterm_cloud_model::testing::{ScriptedPublicationTransport, fixture_snapshot_budget};
    futures::executor::block_on(async {
        let publication = snapshot_publication(store);
        let current = CleanupLease::for_writer(lease.clone());
        let candidate = publication.prepare(current.clone()).await.unwrap();
        let transport = ScriptedPublicationTransport::default();
        loop {
            let after = publication.delivered(stream()).await.unwrap();
            if after >= candidate.anchor().position().event() {
                break;
            }
            let next = publication
                .next(
                    stream(),
                    after,
                    limits(),
                    Arc::new(fixture_snapshot_budget()),
                )
                .await
                .unwrap()
                .unwrap();
            let stored = transport.publish(next).await.unwrap();
            publication
                .record_delivery(current.clone(), stored)
                .await
                .unwrap();
        }
        let stored = publication.pending(stream()).await.unwrap().unwrap();
        let purged = transport.purge(stored).await.unwrap();
        let archive = publication.activate(current, purged.clone()).await.unwrap();
        (archive, purged)
    })
}

pub(crate) fn publication_budgets() -> nocterm_cloud_runner::CoordinatorPublicationBudgets {
    nocterm_cloud_runner::CoordinatorPublicationBudgets::new(
        nocterm_cloud_model::coordinator::CommandReceiveBudget::new(
            nocterm_cloud_model::coordinator::IntakeLimits::default(),
        ),
        nocterm_cloud_model::coordinator::IntakeLimits::default()
            .pool(nocterm_cloud_model::coordinator::IntakeLane::Ordinary),
    )
}
