//! Native capability and transfer adapter integration belongs to app composition.
use nocterm_localfs::LocalFilesystem;
use nocterm_session::{
    RemoteFs, Target,
    testing::{FsOperation, Gate, InMemoryFs},
};
use nocterm_transfer_runtime::{TransferRuntime, TransferRuntimeDeps};
use nocterm_transfers::{
    CollisionPolicy, DownloadRequest, Progress, TransferCause, TransferId, TransferLimits,
    TransferService, TransferState, UploadRequest,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: tempfile::TempDir,
    runtime: Arc<TransferRuntime>,
    remote: Arc<InMemoryFs>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let local = Arc::new(LocalFilesystem::with_base(root.path().to_owned()).unwrap());
        let runtime =
            TransferRuntime::start(TransferRuntimeDeps::new(local, TransferLimits::default()))
                .unwrap();
        let remote = Arc::new(InMemoryFs::default());
        futures::executor::block_on(remote.create_dir("/dest")).unwrap();
        Self {
            root,
            runtime,
            remote,
        }
    }
    fn upload(&self, source: PathBuf, policy: CollisionPolicy) -> TransferId {
        self.runtime
            .enqueue(UploadRequest {
                sources: vec![source],
                target: Target::new("u", "h", 22),
                destination: "/dest".into(),
                fs: self.remote.clone(),
                collisions: policy,
            })
            .unwrap()
    }
    fn download(&self, destination: PathBuf) -> TransferId {
        self.runtime
            .enqueue_download(DownloadRequest {
                sources: vec!["/file".into()],
                target: Target::new("u", "h", 22),
                local_destination: destination,
                fs: self.remote.clone(),
                collisions: CollisionPolicy::Skip,
            })
            .unwrap()
    }
    fn finished(&self, id: TransferId) -> Progress {
        wait(|| {
            self.runtime
                .snapshot()
                .iter()
                .any(|progress| progress.id == id && progress.state.finished())
        });
        self.runtime
            .snapshot()
            .into_iter()
            .find(|progress| progress.id == id)
            .unwrap()
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(Instant::now() < deadline, "Native transfer did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn parts(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".nocterm-")
        })
        .collect()
}
#[test]
fn empty_directory_upload_creates_remote_directory_without_completed_files() {
    let fixture = Fixture::new();
    let source = fixture.root.path().join("empty");
    std::fs::create_dir(&source).unwrap();
    let progress = fixture.finished(fixture.upload(source, CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(progress.discovered_files, 0);
    assert_eq!(progress.prepared_directories, 1);
    assert!(progress.errors.is_empty());
    let metadata = futures::executor::block_on(fixture.remote.metadata("/dest/empty")).unwrap();
    assert_eq!(metadata.kind, nocterm_session::EntryKind::Directory);
}
#[test]
fn empty_directory_download_creates_native_directory_without_completed_files() {
    let fixture = Fixture::new();
    futures::executor::block_on(fixture.remote.create_dir("/empty")).unwrap();
    let destination = fixture.root.path().join("output");
    std::fs::create_dir(&destination).unwrap();
    let id = fixture
        .runtime
        .enqueue_download(DownloadRequest {
            sources: vec!["/empty".into()],
            target: Target::new("u", "h", 22),
            local_destination: destination.clone(),
            fs: fixture.remote.clone(),
            collisions: CollisionPolicy::Skip,
        })
        .unwrap();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(progress.discovered_files, 0);
    assert_eq!(progress.prepared_directories, 1);
    assert!(progress.errors.is_empty());
    assert!(destination.join("empty").is_dir());
    assert!(
        std::fs::read_dir(destination.join("empty"))
            .unwrap()
            .next()
            .is_none()
    );
}
#[cfg(unix)]
#[test]
fn upload_keeps_opened_original_source_when_its_ancestor_is_replaced_by_a_link() {
    let fixture = Fixture::new();
    let source = fixture.root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file"), b"original").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("file"), b"secret").unwrap();
    let gate = Gate::pending();
    fixture
        .remote
        .suspend_next(FsOperation::Upload, "/dest/source/file", gate.clone());
    let id = fixture.upload(source.clone(), CollisionPolicy::Skip);
    wait(|| {
        fixture
            .runtime
            .snapshot()
            .iter()
            .any(|job| job.id == id && job.discovered_files == 1)
    });
    let held = fixture.root.path().join("held");
    std::fs::rename(&source, &held).unwrap();
    std::os::unix::fs::symlink(outside.path(), &source).unwrap();
    gate.release();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(
        fixture.remote.bytes("/dest/source/file").unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(outside.path().join("file")).unwrap(),
        b"secret"
    );
}
#[test]
fn source_growth_or_truncation_during_acknowledgement_refuses_publication() {
    for growth in [false, true] {
        let fixture = Fixture::new();
        let source = fixture.root.path().join("file");
        std::fs::write(&source, vec![1; 100000]).unwrap();
        fixture
            .remote
            .seed_file("/dest/file", b"old-final")
            .unwrap();
        let gate = Gate::pending();
        fixture
            .remote
            .suspend_next(FsOperation::Write, "/dest/file", gate.clone());
        let id = fixture.upload(source.clone(), CollisionPolicy::Replace);
        wait(|| fixture.remote.activity().live_uploads == 1);
        // Wait until a whole bounded batch has been read, before changing the
        // same opened inode whose final metadata the runtime validates.
        let deadline = Instant::now() + Duration::from_secs(10);
        while fixture.remote.activity().requests < 3 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(if growth { 100001 } else { 3 })
            .unwrap();
        gate.release();
        let progress = fixture.finished(id);
        assert_eq!(progress.state, TransferState::Failed);
        assert!(
            progress
                .errors
                .iter()
                .any(|issue| matches!(issue.cause(), TransferCause::SizeChanged { .. }))
        );
        assert_eq!(fixture.remote.bytes("/dest/file").unwrap(), b"old-final");
        assert_eq!(fixture.remote.activity().live_uploads, 0);
    }
}
#[cfg(unix)]
#[test]
fn downloaded_private_stage_publishes_inside_held_parent_after_ancestor_swap() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = Fixture::new();
    let destination = fixture.root.path().join("output");
    std::fs::create_dir(&destination).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fixture.remote.seed_file("/file", b"incoming").unwrap();
    let gate = Gate::pending();
    fixture
        .remote
        .suspend_next(FsOperation::Read, "/file", gate.clone());
    let id = fixture.download(destination.clone());
    wait(|| fixture.remote.activity().live_downloads == 1);
    let temporary = parts(&destination);
    assert_eq!(temporary.len(), 1);
    assert_eq!(
        std::fs::metadata(&temporary[0])
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let held = fixture.root.path().join("held");
    std::fs::rename(&destination, &held).unwrap();
    std::os::unix::fs::symlink(outside.path(), &destination).unwrap();
    gate.release();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(std::fs::read(held.join("file")).unwrap(), b"incoming");
    assert!(!outside.path().join("file").exists());
    assert!(parts(&held).is_empty());
}
#[test]
fn canceled_native_download_removes_owned_private_stage_and_keeps_other_files() {
    let fixture = Fixture::new();
    let destination = fixture.root.path().join("output");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("other"), b"keep").unwrap();
    fixture.remote.seed_file("/file", vec![1; 65536]).unwrap();
    fixture
        .remote
        .suspend_next(FsOperation::Read, "/file", Gate::pending());
    let id = fixture.download(destination.clone());
    wait(|| fixture.remote.activity().live_downloads == 1);
    assert_eq!(parts(&destination).len(), 1);
    fixture.runtime.cancel(id).unwrap();
    assert_eq!(fixture.finished(id).state, TransferState::Cancelled);
    assert!(parts(&destination).is_empty());
    assert!(!destination.join("file").exists());
    assert_eq!(std::fs::read(destination.join("other")).unwrap(), b"keep");
    assert_eq!(fixture.remote.activity().live_downloads, 0);
}
