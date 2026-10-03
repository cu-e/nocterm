//! User-facing batches exercised against a staged filesystem with observable
//! I/O resource ownership, failure and back-pressure.

use nocterm_session::{
    DirEntry, EntryKind, FsError, FsFuture, RemoteFs, Target,
    fs::{RemoteUpload, UploadMode},
};
use nocterm_transfers::{
    CollisionPolicy, Progress, QueueError, TransferState, Transfers, UploadRequest,
};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    dirs: Mutex<HashSet<String>>,
    live: AtomicUsize,
    max_live: AtomicUsize,
    max_chunk: AtomicUsize,
    max_batch: AtomicUsize,
    acked_bytes: AtomicUsize,
    cancelled: AtomicUsize,
    stall: AtomicBool,
    stat_stall: AtomicBool,
    fail: AtomicBool,
}
#[derive(Clone, Default)]
struct Fs(Arc<State>);
fn entry(name: &str, kind: EntryKind, size: u64) -> DirEntry {
    DirEntry {
        name: name.into(),
        kind,
        is_symlink: false,
        size: Some(size),
    }
}
impl RemoteFs for Fs {
    fn home(&self) -> FsFuture<String> {
        Box::pin(async { Ok("/dest".into()) })
    }
    fn read_dir(&self, _: &str) -> FsFuture<Vec<DirEntry>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
        let state = self.0.clone();
        let path = path.to_owned();
        Box::pin(async move {
            while state.stat_stall.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Ok(state
                .files
                .lock()
                .unwrap()
                .get(&path)
                .map(|bytes| entry(&path, EntryKind::File, bytes.len() as u64)))
        })
    }
    fn create_dir(&self, path: &str) -> FsFuture<()> {
        let state = self.0.clone();
        let path = path.to_owned();
        Box::pin(async move {
            state.dirs.lock().unwrap().insert(path);
            Ok(())
        })
    }
    fn upload(&self, path: &str, mode: UploadMode) -> FsFuture<Box<dyn RemoteUpload>> {
        let state = self.0.clone();
        let path = path.to_owned();
        Box::pin(async move {
            if mode == UploadMode::Create && state.files.lock().unwrap().contains_key(&path) {
                return Err(FsError::AlreadyExists { path });
            }
            let live = state.live.fetch_add(1, Ordering::SeqCst) + 1;
            state.max_live.fetch_max(live, Ordering::SeqCst);
            Ok(Box::new(Writer {
                state,
                path,
                mode,
                bytes: Arc::default(),
                published: false,
            }) as Box<dyn RemoteUpload>)
        })
    }
}
struct Writer {
    state: Arc<State>,
    path: String,
    mode: UploadMode,
    bytes: Arc<Mutex<Vec<u8>>>,
    published: bool,
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.state.live.fetch_sub(1, Ordering::SeqCst);
        if !self.published {
            self.state.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}
impl RemoteUpload for Writer {
    fn write(&mut self, bytes: Vec<u8>) -> FsFuture<()> {
        let state = self.state.clone();
        let destination = self.bytes.clone();
        let path = self.path.clone();
        Box::pin(async move {
            state.max_chunk.fetch_max(bytes.len(), Ordering::SeqCst);
            while state.stall.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            tokio::time::sleep(Duration::from_millis(3)).await;
            if state.fail.load(Ordering::SeqCst) {
                return Err(FsError::PermissionDenied { path });
            }
            state.acked_bytes.fetch_add(bytes.len(), Ordering::SeqCst);
            destination.lock().unwrap().extend(bytes);
            Ok(())
        })
    }
    fn write_batch(&mut self, chunks: Vec<Vec<u8>>) -> FsFuture<()> {
        self.state
            .max_batch
            .fetch_max(chunks.len(), Ordering::SeqCst);
        assert!(chunks.len() <= 8);
        assert!(chunks.iter().all(|chunk| chunk.len() <= 32 * 1024));
        let writes: Vec<_> = chunks.into_iter().map(|chunk| self.write(chunk)).collect();
        Box::pin(async move {
            for write in writes {
                write.await?;
            }
            Ok(())
        })
    }
    fn finish(mut self: Box<Self>) -> FsFuture<String> {
        Box::pin(async move {
            {
                let mut files = self.state.files.lock().unwrap();
                if self.mode == UploadMode::Create && files.contains_key(&self.path) {
                    return Err(FsError::AlreadyExists {
                        path: self.path.clone(),
                    });
                }
                files.insert(self.path.clone(), self.bytes.lock().unwrap().clone());
            }
            self.published = true;
            Ok(self.path.clone())
        })
    }
    fn cancel(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async move {
            drop(self);
            Ok(())
        })
    }
}
fn request(fs: &Fs, sources: Vec<PathBuf>, policy: CollisionPolicy) -> UploadRequest {
    UploadRequest {
        sources,
        target: Target::new("user", "fixed-host", 2222),
        destination: "/dest".into(),
        fs: Arc::new(fs.clone()),
        collisions: policy,
    }
}
fn wait(service: &Transfers, id: uuid::Uuid, predicate: impl Fn(&Progress) -> bool) -> Progress {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let p = service.snapshot().into_iter().find(|p| p.id == id).unwrap();
        if predicate(&p) {
            return p;
        }
        assert!(Instant::now() < deadline, "batch stalled: {p:?}");
        thread::sleep(Duration::from_millis(5));
    }
}
fn finished(service: &Transfers, id: uuid::Uuid) -> Progress {
    wait(service, id, |p| p.state.finished())
}

#[test]
fn many_files_stream_with_four_writers_and_bounded_chunks() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("tree");
    std::fs::create_dir(&source).unwrap();
    for n in 0..120 {
        std::fs::write(source.join(format!("file-{n}.bin")), vec![n as u8; 83]).unwrap();
    }
    std::fs::write(source.join("large.bin"), vec![0x91; 800_123]).unwrap();
    let fs = Fs::default();
    let service = Transfers::new().unwrap();
    let id = service
        .enqueue(request(&fs, vec![source], CollisionPolicy::Skip))
        .unwrap();
    let p = finished(&service, id);
    assert_eq!(p.state, TransferState::Completed);
    assert_eq!(p.completed_files, 121);
    assert_eq!(p.sent_bytes, p.total_bytes);
    assert_eq!(p.failed_files, 0);
    assert!(p.discovery_complete);
    assert_eq!(fs.0.files.lock().unwrap().len(), 121);
    let max = fs.0.max_live.load(Ordering::SeqCst);
    assert!((2..=4).contains(&max), "concurrency was {max}");
    assert_eq!(fs.0.max_chunk.load(Ordering::SeqCst), 32 * 1024);
    assert_eq!(fs.0.max_batch.load(Ordering::SeqCst), 8);
    assert_eq!(p.sent_bytes, fs.0.acked_bytes.load(Ordering::SeqCst) as u64);
    assert_eq!(fs.0.live.load(Ordering::SeqCst), 0);
}
#[test]
fn collision_choices_are_explicit_and_do_not_modify_existing_content() {
    for policy in [
        CollisionPolicy::Skip,
        CollisionPolicy::Rename,
        CollisionPolicy::Replace,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("report.txt");
        std::fs::write(&source, b"new").unwrap();
        let fs = Fs::default();
        fs.0.files
            .lock()
            .unwrap()
            .insert("/dest/report.txt".into(), b"old".to_vec());
        let service = Transfers::new().unwrap();
        let p = finished(
            &service,
            service.enqueue(request(&fs, vec![source], policy)).unwrap(),
        );
        assert_eq!(p.state, TransferState::Completed);
        let files = fs.0.files.lock().unwrap();
        match policy {
            CollisionPolicy::Skip => {
                assert_eq!(p.skipped_files, 1);
                assert_eq!(files.get("/dest/report.txt").unwrap(), b"old");
            }
            CollisionPolicy::Rename => {
                assert_eq!(files.get("/dest/report.txt").unwrap(), b"old");
                assert_eq!(files.get("/dest/report (1).txt").unwrap(), b"new");
            }
            CollisionPolicy::Replace => assert_eq!(files.get("/dest/report.txt").unwrap(), b"new"),
        }
    }
}
#[cfg(unix)]
#[test]
fn directory_discovery_does_not_follow_symlinks_or_cycles() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("tree");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    std::fs::write(source.join("sub/file"), b"inside").unwrap();
    std::os::unix::fs::symlink(&source, source.join("sub/cycle")).unwrap();
    std::os::unix::fs::symlink(source.join("sub/file"), source.join("link")).unwrap();
    let fs = Fs::default();
    let service = Transfers::new().unwrap();
    let p = finished(
        &service,
        service
            .enqueue(request(&fs, vec![source], CollisionPolicy::Skip))
            .unwrap(),
    );
    assert_eq!(p.state, TransferState::Completed);
    assert_eq!(p.completed_files, 1);
    assert_eq!(p.skipped_files, 2);
    assert_eq!(fs.0.files.lock().unwrap().len(), 1);
    assert_eq!(p.errors.len(), 2);
}
#[test]
fn cancellation_aborts_stalled_writes_after_discovery_and_cleans_staging() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("file");
    std::fs::write(&source, b"bytes").unwrap();
    let fs = Fs::default();
    fs.0.stall.store(true, Ordering::SeqCst);
    let service = Transfers::new().unwrap();
    let id = service
        .enqueue(request(&fs, vec![source], CollisionPolicy::Skip))
        .unwrap();
    wait(&service, id, |p| {
        p.discovery_complete && fs.0.live.load(Ordering::SeqCst) > 0
    });
    service.cancel(id);
    assert_eq!(finished(&service, id).state, TransferState::Cancelled);
    assert_eq!(fs.0.live.load(Ordering::SeqCst), 0);
    assert!(fs.0.files.lock().unwrap().is_empty());
    assert_eq!(fs.0.cancelled.load(Ordering::SeqCst), 1);
}
#[test]
fn failure_details_are_bounded_and_retry_keeps_target_and_destination() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("tree");
    std::fs::create_dir(&source).unwrap();
    for n in 0..125 {
        std::fs::write(source.join(format!("{n}")), b"x").unwrap();
    }
    let fs = Fs::default();
    fs.0.fail.store(true, Ordering::SeqCst);
    let service = Transfers::new().unwrap();
    let id = service
        .enqueue(request(&fs, vec![source], CollisionPolicy::Skip))
        .unwrap();
    let p = finished(&service, id);
    assert_eq!(p.state, TransferState::Failed);
    assert_eq!(p.failed_files, 125);
    assert_eq!(p.errors.len(), 100);
    assert_eq!(fs.0.live.load(Ordering::SeqCst), 0);
    assert_eq!(fs.0.cancelled.load(Ordering::SeqCst), 125);
    let next_fs = Fs::default();
    let retry = service.retry(id, Arc::new(next_fs.clone())).unwrap();
    let p2 = finished(&service, retry);
    assert_eq!(p2.state, TransferState::Completed);
    assert_eq!(p2.target, p.target);
    assert_eq!(p2.destination, p.destination);
    assert_eq!(p2.completed_files, 125);
    assert!(fs.0.files.lock().unwrap().is_empty());
}
#[test]
fn queue_backpressure_is_visible_and_queued_cancel_never_opens_a_writer() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("file");
    std::fs::write(&source, b"x").unwrap();
    let fs = Fs::default();
    fs.0.stall.store(true, Ordering::SeqCst);
    let service = Transfers::new().unwrap();
    let first = service
        .enqueue(request(&fs, vec![source.clone()], CollisionPolicy::Skip))
        .unwrap();
    wait(&service, first, |_| fs.0.live.load(Ordering::SeqCst) > 0);
    let mut queued = vec![];
    for _ in 0..8 {
        queued.push(
            service
                .enqueue(request(&fs, vec![source.clone()], CollisionPolicy::Skip))
                .unwrap(),
        );
    }
    assert!(matches!(
        service.enqueue(request(&fs, vec![source], CollisionPolicy::Skip)),
        Err(QueueError::Full)
    ));
    for id in &queued {
        service.cancel(*id);
    }
    service.cancel(first);
    finished(&service, first);
    for id in queued {
        assert_eq!(finished(&service, id).state, TransferState::Cancelled);
    }
    assert_eq!(fs.0.max_live.load(Ordering::SeqCst), 1);
}

#[test]
fn rename_policy_handles_same_name_sources_without_racing_publication() {
    let directory = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    for (folder, contents) in [("a", b"first".as_slice()), ("b", b"second".as_slice())] {
        std::fs::create_dir(directory.path().join(folder)).unwrap();
        let path = directory.path().join(folder).join("same.txt");
        std::fs::write(&path, contents).unwrap();
        sources.push(path);
    }
    let fs = Fs::default();
    let service = Transfers::new().unwrap();
    let progress = finished(
        &service,
        service
            .enqueue(request(&fs, sources, CollisionPolicy::Rename))
            .unwrap(),
    );
    assert_eq!(
        progress.state,
        TransferState::Completed,
        "Rename must preserve both selected files: {progress:?}"
    );
    assert_eq!(progress.completed_files, 2);
    let mut contents: Vec<_> = fs.0.files.lock().unwrap().values().cloned().collect();
    contents.sort();
    assert_eq!(contents, vec![b"first".to_vec(), b"second".to_vec()]);
}

#[test]
fn safe_default_skips_overlapping_selected_names_without_failing_batch() {
    let directory = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    for folder in ["a", "b"] {
        std::fs::create_dir(directory.path().join(folder)).unwrap();
        let path = directory.path().join(folder).join("same.txt");
        std::fs::write(&path, b"content").unwrap();
        sources.push(path);
    }
    let fs = Fs::default();
    let service = Transfers::new().unwrap();
    let progress = finished(
        &service,
        service
            .enqueue(request(&fs, sources, CollisionPolicy::Skip))
            .unwrap(),
    );
    assert_eq!(
        progress.state,
        TransferState::Completed,
        "default Skip must preserve one file and skip its duplicate: {progress:?}"
    );
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.skipped_files, 1);
    assert_eq!(progress.failed_files, 0);
}

#[cfg(unix)]
#[test]
fn upload_rejects_ancestor_replaced_with_a_link_after_discovery() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let parent = root.path().join("source");
    std::fs::create_dir(&parent).unwrap();
    let source = parent.join("file");
    std::fs::write(&source, b"selected").unwrap();
    std::fs::write(outside.path().join("file"), b"private unselected data").unwrap();
    let fs = Fs::default();
    fs.0.stat_stall.store(true, Ordering::SeqCst);
    let service = Transfers::new().unwrap();
    let id = service
        .enqueue(request(&fs, vec![source], CollisionPolicy::Skip))
        .unwrap();
    wait(&service, id, |p| p.discovery_complete);
    std::fs::rename(&parent, root.path().join("original")).unwrap();
    symlink(outside.path(), &parent).unwrap();
    fs.0.stat_stall.store(false, Ordering::SeqCst);
    assert_eq!(finished(&service, id).failed_files, 1);
    assert!(fs.0.files.lock().unwrap().is_empty());
    assert_eq!(fs.0.max_live.load(Ordering::SeqCst), 0);
}

#[test]
fn local_size_changes_during_acknowledgement_never_publish_partial_content() {
    for new_size in [100_000, 900_000] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("file");
        std::fs::write(&source, vec![3; 500_000]).unwrap();
        let fs = Fs::default();
        fs.0.files
            .lock()
            .unwrap()
            .insert("/dest/file".into(), b"original".to_vec());
        fs.0.stall.store(true, Ordering::SeqCst);
        let service = Transfers::new().unwrap();
        let id = service
            .enqueue(request(&fs, vec![source.clone()], CollisionPolicy::Replace))
            .unwrap();
        wait(&service, id, |p| {
            p.discovery_complete && fs.0.live.load(Ordering::SeqCst) == 1
        });
        assert_eq!(
            service
                .snapshot()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap()
                .sent_bytes,
            0
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(source)
            .unwrap()
            .set_len(new_size)
            .unwrap();
        fs.0.stall.store(false, Ordering::SeqCst);
        let p = finished(&service, id);
        assert_eq!(p.state, TransferState::Failed);
        assert_eq!(
            fs.0.files.lock().unwrap().get("/dest/file").unwrap(),
            b"original"
        );
        assert_eq!(fs.0.live.load(Ordering::SeqCst), 0);
        assert_eq!(fs.0.cancelled.load(Ordering::SeqCst), 1);
    }
}
