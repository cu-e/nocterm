//! Behavior of bounded downloads, publication, cancellation and hostile trees.
use nocterm_session::{
    DirEntry, EntryKind, FsError, FsFuture, RemoteFs, Target, fs::RemoteDownload,
};
use nocterm_transfers::{
    CollisionPolicy, DownloadRequest, Progress, TransferDirection, TransferState, Transfers,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    dirs: Mutex<BTreeMap<String, Vec<DirEntry>>>,
    live: AtomicUsize,
    peak: AtomicUsize,
    max_read: AtomicUsize,
    closed: AtomicUsize,
    stall: AtomicBool,
    fail: AtomicBool,
    oversize: AtomicBool,
    declared_size: Mutex<Option<Option<u64>>>,
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
impl Fs {
    fn file(&self, path: &str, bytes: Vec<u8>) {
        self.0.files.lock().unwrap().insert(path.into(), bytes);
    }
    fn dir(&self, path: &str, entries: Vec<DirEntry>) {
        self.0.dirs.lock().unwrap().insert(path.into(), entries);
    }
}
impl RemoteFs for Fs {
    fn home(&self) -> FsFuture<String> {
        Box::pin(async { Ok("/".into()) })
    }
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        let entries = self
            .0
            .dirs
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(entries) })
    }
    fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
        let result = if let Some(bytes) = self.0.files.lock().unwrap().get(path) {
            Some(entry(path, EntryKind::File, bytes.len() as u64))
        } else if self.0.dirs.lock().unwrap().contains_key(path) {
            Some(entry(path, EntryKind::Directory, 0))
        } else {
            None
        };
        let result = result.map(|mut entry| {
            if let Some(size) = *self.0.declared_size.lock().unwrap() {
                entry.size = size;
            }
            entry
        });
        Box::pin(async move { Ok(result) })
    }
    fn download(&self, path: &str) -> FsFuture<Box<dyn RemoteDownload>> {
        let state = self.0.clone();
        let path = path.to_owned();
        Box::pin(async move {
            let bytes = state
                .files
                .lock()
                .unwrap()
                .get(&path)
                .cloned()
                .ok_or(FsError::NotFound { path })?;
            let live = state.live.fetch_add(1, Ordering::SeqCst) + 1;
            state.peak.fetch_max(live, Ordering::SeqCst);
            Ok(Box::new(Reader {
                state,
                bytes: Arc::new(bytes),
                offset: 0,
            }) as Box<dyn RemoteDownload>)
        })
    }
}
struct Reader {
    state: Arc<State>,
    bytes: Arc<Vec<u8>>,
    offset: usize,
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.state.live.fetch_sub(1, Ordering::SeqCst);
    }
}
impl RemoteDownload for Reader {
    fn read(&mut self, max: usize) -> FsFuture<Vec<u8>> {
        self.state.max_read.fetch_max(max, Ordering::SeqCst);
        let end = (self.offset + max).min(self.bytes.len());
        let bytes = self.bytes[self.offset..end].to_vec();
        self.offset = end;
        let state = self.state.clone();
        Box::pin(async move {
            while state.stall.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
            if state.fail.load(Ordering::SeqCst) {
                return Err(FsError::Other("read failed".into()));
            }
            if state.oversize.load(Ordering::SeqCst) {
                return Ok(vec![0; max + 1]);
            }
            Ok(bytes)
        })
    }
    fn close(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async move {
            self.state.closed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
fn target(host: &str) -> Target {
    Target {
        host: host.into(),
        port: 22,
        user: "me".into(),
    }
}
fn enqueue(
    transfers: &Transfers,
    fs: &Fs,
    root: &Path,
    sources: &[&str],
    collisions: CollisionPolicy,
) -> uuid::Uuid {
    transfers
        .enqueue_download(DownloadRequest {
            sources: sources.iter().map(|s| s.to_string()).collect(),
            target: target("pinned-host"),
            local_destination: root.to_owned(),
            fs: Arc::new(fs.clone()),
            collisions,
        })
        .unwrap()
}
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn finish(transfers: &Transfers, id: uuid::Uuid) -> Progress {
    wait_for(|| {
        transfers
            .snapshot()
            .iter()
            .any(|p| p.id == id && p.state.finished())
    });
    transfers
        .snapshot()
        .into_iter()
        .find(|p| p.id == id)
        .unwrap()
}
fn assert_no_parts(root: &Path) {
    assert!(std::fs::read_dir(root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));
}

#[test]
fn many_files_stream_with_bounded_readers_and_exact_bytes() {
    let root = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    let names: Vec<String> = (0..120).map(|i| format!("/f{i}")).collect();
    for (i, name) in names.iter().enumerate() {
        fs.file(name, vec![i as u8; if i == 0 { 1_000_003 } else { 17 }]);
    }
    let sources: Vec<&str> = names.iter().map(String::as_str).collect();
    let p = finish(
        &transfers,
        enqueue(
            &transfers,
            &fs,
            root.path(),
            &sources,
            CollisionPolicy::Skip,
        ),
    );
    assert_eq!(p.state, TransferState::Completed);
    assert_eq!(p.direction, TransferDirection::Download);
    assert_eq!(p.completed_files, 120);
    assert_eq!(p.sent_bytes, p.total_bytes);
    assert_eq!(fs.0.max_read.load(Ordering::SeqCst), 32 * 1024);
    assert_eq!(fs.0.peak.load(Ordering::SeqCst), 4);
    assert_eq!(fs.0.closed.load(Ordering::SeqCst), 120);
    assert_eq!(fs.0.live.load(Ordering::SeqCst), 0);
    for name in &names {
        assert_eq!(
            std::fs::read(root.path().join(&name[1..])).unwrap(),
            *fs.0.files.lock().unwrap().get(name).unwrap()
        );
    }
    assert_no_parts(root.path());
}
#[test]
fn publication_preserves_old_bytes_until_completed_and_obeys_collision_policy() {
    let root = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    fs.file("/report.txt", b"new".to_vec());
    std::fs::write(root.path().join("report.txt"), b"old").unwrap();
    let p = finish(
        &transfers,
        enqueue(
            &transfers,
            &fs,
            root.path(),
            &["/report.txt"],
            CollisionPolicy::Skip,
        ),
    );
    assert_eq!(p.skipped_files, 1);
    assert_eq!(fs.0.closed.load(Ordering::SeqCst), 0);
    let p = finish(
        &transfers,
        enqueue(
            &transfers,
            &fs,
            root.path(),
            &["/report.txt"],
            CollisionPolicy::Rename,
        ),
    );
    assert_eq!(p.completed_files, 1);
    assert_eq!(
        std::fs::read(root.path().join("report (1).txt")).unwrap(),
        b"new"
    );
    fs.0.stall.store(true, Ordering::SeqCst);
    let id = enqueue(
        &transfers,
        &fs,
        root.path(),
        &["/report.txt"],
        CollisionPolicy::Replace,
    );
    wait_for(|| fs.0.live.load(Ordering::SeqCst) == 1);
    assert_eq!(
        std::fs::read(root.path().join("report.txt")).unwrap(),
        b"old"
    );
    fs.0.stall.store(false, Ordering::SeqCst);
    assert_eq!(finish(&transfers, id).completed_files, 1);
    assert_eq!(
        std::fs::read(root.path().join("report.txt")).unwrap(),
        b"new"
    );
    assert_no_parts(root.path());
}
#[test]
fn recursive_empty_directories_are_preserved_and_hostile_entries_are_visible() {
    let root = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    let mut link = entry("link", EntryKind::Directory, 0);
    link.is_symlink = true;
    fs.dir(
        "/tree",
        vec![
            entry("empty", EntryKind::Directory, 0),
            entry("ok", EntryKind::File, 4),
            link,
            entry("../escaped", EntryKind::File, 4),
            entry("C:drive", EntryKind::File, 4),
            entry("socket", EntryKind::Other, 0),
        ],
    );
    fs.dir("/tree/empty", vec![]);
    fs.file("/tree/ok", b"data".to_vec());
    let p = finish(
        &transfers,
        enqueue(
            &transfers,
            &fs,
            root.path(),
            &["/tree"],
            CollisionPolicy::Skip,
        ),
    );
    assert_eq!(p.completed_files, 1);
    assert_eq!(p.skipped_files, 2);
    assert_eq!(p.failed_files, 2);
    assert!(root.path().join("tree/empty").is_dir());
    assert_eq!(std::fs::read(root.path().join("tree/ok")).unwrap(), b"data");
    assert!(!root.path().join("escaped").exists());
}
#[test]
fn cancel_and_read_errors_discard_private_partial_and_release_reader() {
    let root = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    fs.file("/f", vec![3; 100_000]);
    fs.0.stall.store(true, Ordering::SeqCst);
    let id = enqueue(&transfers, &fs, root.path(), &["/f"], CollisionPolicy::Skip);
    wait_for(|| fs.0.live.load(Ordering::SeqCst) == 1);
    transfers.cancel(id);
    assert_eq!(finish(&transfers, id).state, TransferState::Cancelled);
    assert_eq!(fs.0.live.load(Ordering::SeqCst), 0);
    assert!(!root.path().join("f").exists());
    assert_no_parts(root.path());
    fs.0.stall.store(false, Ordering::SeqCst);
    fs.0.fail.store(true, Ordering::SeqCst);
    let p = finish(
        &transfers,
        enqueue(&transfers, &fs, root.path(), &["/f"], CollisionPolicy::Skip),
    );
    assert_eq!(p.state, TransferState::Failed);
    assert_no_parts(root.path());
    let recovered = Fs::default();
    recovered.file("/f", b"retry".to_vec());
    let retry = transfers.retry(p.id, Arc::new(recovered)).unwrap();
    let p = finish(&transfers, retry);
    assert_eq!(p.target, target("pinned-host"));
    assert_eq!(p.destination, root.path().display().to_string());
    assert_eq!(std::fs::read(root.path().join("f")).unwrap(), b"retry");
}
#[test]
fn same_basename_files_do_not_race_their_reserved_destinations() {
    for collisions in [CollisionPolicy::Skip, CollisionPolicy::Rename] {
        let root = tempfile::tempdir().unwrap();
        let fs = Fs::default();
        let transfers = Transfers::new().unwrap();
        fs.file("/a/file", vec![1; 80_000]);
        fs.file("/b/file", vec![2; 80_000]);
        let p = finish(
            &transfers,
            enqueue(
                &transfers,
                &fs,
                root.path(),
                &["/a/file", "/b/file"],
                collisions,
            ),
        );
        assert_eq!(p.failed_files, 0);
        if collisions == CollisionPolicy::Skip {
            assert_eq!(p.completed_files, 1);
            assert_eq!(p.skipped_files, 1);
        } else {
            assert_eq!(p.completed_files, 2);
            let mut bytes = vec![
                std::fs::read(root.path().join("file")).unwrap(),
                std::fs::read(root.path().join("file (1)")).unwrap(),
            ];
            bytes.sort();
            assert_eq!(bytes, vec![vec![1; 80_000], vec![2; 80_000]]);
        }
        assert_no_parts(root.path());
    }
}
#[test]
fn hostile_oversized_reader_is_rejected_without_publication() {
    let root = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    fs.file("/f", vec![1; 10]);
    fs.0.oversize.store(true, Ordering::SeqCst);
    let p = finish(
        &transfers,
        enqueue(&transfers, &fs, root.path(), &["/f"], CollisionPolicy::Skip),
    );
    assert_eq!(p.failed_files, 1);
    assert_eq!(p.sent_bytes, 0);
    assert!(!root.path().join("f").exists());
    assert_no_parts(root.path());
}
#[test]
fn traversal_source_is_rejected_before_queueing() {
    let root = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    for source in ["relative", "/a/../f", "/a/./f", "/", "/C:escape", "/a\\b"] {
        assert!(
            transfers
                .enqueue_download(DownloadRequest {
                    sources: vec![source.into()],
                    target: target("host"),
                    local_destination: root.path().into(),
                    fs: Arc::new(fs.clone()),
                    collisions: CollisionPolicy::Skip
                })
                .is_err()
        );
    }
    assert!(transfers.snapshot().is_empty());
}
#[cfg(unix)]
#[test]
fn destination_links_are_rejected_and_ancestor_swap_cannot_escape_capability() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    fs.file("/tree/f", vec![2; 100]);
    fs.dir("/tree", vec![entry("f", EntryKind::File, 100)]);
    symlink(outside.path(), root.path().join("tree")).unwrap();
    let p = finish(
        &transfers,
        enqueue(
            &transfers,
            &fs,
            root.path(),
            &["/tree"],
            CollisionPolicy::Replace,
        ),
    );
    assert_eq!(p.failed_files, 1);
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    std::fs::remove_file(root.path().join("tree")).unwrap();
    let destination = root.path().join("real");
    std::fs::create_dir(&destination).unwrap();
    fs.file("/f", vec![4; 100]);
    fs.0.stall.store(true, Ordering::SeqCst);
    let id = enqueue(
        &transfers,
        &fs,
        &destination,
        &["/f"],
        CollisionPolicy::Skip,
    );
    wait_for(|| fs.0.live.load(Ordering::SeqCst) == 1);
    std::fs::rename(&destination, root.path().join("held")).unwrap();
    symlink(outside.path(), &destination).unwrap();
    fs.0.stall.store(false, Ordering::SeqCst);
    assert_eq!(finish(&transfers, id).completed_files, 1);
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    assert_eq!(
        std::fs::read(root.path().join("held/f")).unwrap(),
        vec![4; 100]
    );
    assert_no_parts(&root.path().join("held"));
}

#[test]
fn declared_size_changes_refuse_publication_but_unknown_size_can_stream() {
    for declared in [Some(2), Some(100), None] {
        let root = tempfile::tempdir().unwrap();
        let fs = Fs::default();
        let transfers = Transfers::new().unwrap();
        fs.file("/f", b"actual".to_vec());
        *fs.0.declared_size.lock().unwrap() = Some(declared);
        std::fs::write(root.path().join("f"), b"original").unwrap();
        let p = finish(
            &transfers,
            enqueue(
                &transfers,
                &fs,
                root.path(),
                &["/f"],
                CollisionPolicy::Replace,
            ),
        );
        if declared.is_some() {
            assert_eq!(p.state, TransferState::Failed);
            assert_eq!(std::fs::read(root.path().join("f")).unwrap(), b"original");
        } else {
            assert_eq!(p.state, TransferState::Completed);
            assert_eq!(std::fs::read(root.path().join("f")).unwrap(), b"actual");
        }
        assert_no_parts(root.path());
    }
}
#[test]
fn pathological_tree_depth_is_capped_and_error_details_remain_bounded() {
    let root = tempfile::tempdir().unwrap();
    let fs = Fs::default();
    let transfers = Transfers::new().unwrap();
    let mut path = "/d".to_owned();
    for _ in 0..260 {
        fs.dir(&path, vec![entry("d", EntryKind::Directory, 0)]);
        path.push_str("/d");
    }
    let p = finish(
        &transfers,
        enqueue(&transfers, &fs, root.path(), &["/d"], CollisionPolicy::Skip),
    );
    assert_eq!(p.failed_files, 1);
    assert!(p.errors[0].contains("256"));
    let sources: Vec<String> = (0..125).map(|i| format!("/missing{i}")).collect();
    let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
    let p = finish(
        &transfers,
        enqueue(
            &transfers,
            &fs,
            root.path(),
            &sources,
            CollisionPolicy::Skip,
        ),
    );
    assert_eq!(p.failed_files, 125);
    assert_eq!(p.errors.len(), 100);
}

#[test]
fn late_local_collision_skips_or_renames_without_clobbering_external_file() {
    for policy in [CollisionPolicy::Skip, CollisionPolicy::Rename] {
        let root = tempfile::tempdir().unwrap();
        let fs = Fs::default();
        let transfers = Transfers::new().unwrap();
        fs.file("/f", b"selected".to_vec());
        fs.0.stall.store(true, Ordering::SeqCst);
        let id = enqueue(&transfers, &fs, root.path(), &["/f"], policy);
        wait_for(|| fs.0.live.load(Ordering::SeqCst) == 1);
        std::fs::write(root.path().join("f"), b"external").unwrap();
        fs.0.stall.store(false, Ordering::SeqCst);
        let p = finish(&transfers, id);
        assert_eq!(p.state, TransferState::Completed);
        assert_eq!(std::fs::read(root.path().join("f")).unwrap(), b"external");
        if policy == CollisionPolicy::Skip {
            assert_eq!(p.skipped_files, 1);
        } else {
            assert_eq!(p.completed_files, 1);
            assert_eq!(
                std::fs::read(root.path().join("f (1)")).unwrap(),
                b"selected"
            );
        }
        assert_no_parts(root.path());
    }
}
