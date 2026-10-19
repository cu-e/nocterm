use super::*;
use futures::executor::block_on;
use std::{collections::VecDeque, error::Error};
fn native_cause(error: &RecordingError) -> &io::Error {
    error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap()
}
fn block(sequence: u64, offset: u64, bytes: &[u8]) -> AppendBlock {
    AppendBlock::new(sequence, offset, Arc::from(bytes)).unwrap()
}
#[test]
fn exclusive_creation_keeps_existing_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output.log");
    fs::write(&path, b"existing").unwrap();
    let error = create_at(path.clone(), 20).err().unwrap();
    assert!(matches!(error, RecordingError::Create { .. }));
    let native = native_cause(&error);
    assert_eq!(native.kind(), io::ErrorKind::AlreadyExists);
    // std::io adapts FileError by forwarding its source, not exposing a FileError frame.
    let deeper = native
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(deeper.kind(), io::ErrorKind::AlreadyExists);
    assert!(deeper.raw_os_error().is_some());
    let clone = error.clone();
    assert!(std::ptr::eq(native, native_cause(&clone)));
    assert_eq!(fs::read(path).unwrap(), b"existing");
}
#[test]
fn append_is_pinned_and_exact_duplicates_do_not_repeat_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output.log");
    let original = directory.path().join("original.log");
    let log = create_at(path.clone(), 20).unwrap();
    fs::rename(&path, &original).unwrap();
    fs::write(&path, b"replacement").unwrap();
    let first = block(1, 0, b"hello");
    assert!(block_on(log.append(first.clone())).error().is_none());
    assert!(block_on(log.append(first)).error().is_none());
    let sealed = block_on(log.seal());
    assert!(sealed.prefix().synced());
    assert!(sealed.prefix().closed());
    assert_eq!(fs::read(original).unwrap(), b"hello");
    assert_eq!(fs::read(path).unwrap(), b"replacement");
    assert!(matches!(
        block_on(log.append(block(2, 5, b"!"))).error(),
        Some(RecordingError::Closing)
    ));
}
#[cfg(unix)]
#[test]
fn private_permissions_and_final_links_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    fs::write(&target, b"untouched").unwrap();
    for (name, destination) in [
        ("existing-link", target.clone()),
        ("dangling-link", directory.path().join("absent")),
    ] {
        let path = directory.path().join(name);
        symlink(destination, &path).unwrap();
        assert!(create_at(path, 20).is_err());
    }
    assert_eq!(fs::read(target).unwrap(), b"untouched");
    let path = directory.path().join("private.log");
    let log = create_at(path.clone(), 20).unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    block_on(log.seal());
}
#[test]
fn cap_rejects_whole_crossing_block_and_seals_previous_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bounded.log");
    let log = create_at(path.clone(), 5).unwrap();
    block_on(log.append(block(1, 0, b"1234")));
    let result = block_on(log.append(block(2, 4, b"56")));
    assert!(matches!(result.error(), Some(RecordingError::Limit)));
    assert_eq!(result.prefix().written_bytes(), 4);
    assert_eq!(block_on(log.seal()).prefix().written_bytes(), 4);
    assert_eq!(fs::read(path).unwrap(), b"1234");
}
struct ScriptWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    steps: VecDeque<io::Result<usize>>,
    flush_error: bool,
    sync_error: bool,
    syncs: Arc<Mutex<usize>>,
    drops: Arc<Mutex<usize>>,
}
impl Write for ScriptWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self
            .steps
            .pop_front()
            .unwrap_or(Ok(bytes.len()))?
            .min(bytes.len());
        lock(&self.bytes).extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.flush_error {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        }
    }
}
impl DurableWrite for ScriptWriter {
    fn sync(&mut self) -> io::Result<()> {
        *lock(&self.syncs) += 1;
        if self.sync_error {
            Err(io::ErrorKind::Other.into())
        } else {
            Ok(())
        }
    }
}
impl Drop for ScriptWriter {
    fn drop(&mut self) {
        *lock(&self.drops) += 1;
    }
}
fn writer(steps: Vec<io::Result<usize>>, flush_error: bool, sync_error: bool) -> ScriptWriter {
    ScriptWriter {
        bytes: Arc::default(),
        steps: steps.into(),
        flush_error,
        sync_error,
        syncs: Arc::default(),
        drops: Arc::default(),
    }
}
#[test]
fn interrupted_short_writes_count_exact_partial_prefix_without_retry() {
    let writer = writer(
        vec![
            Err(io::ErrorKind::Interrupted.into()),
            Ok(2),
            Err(io::ErrorKind::StorageFull.into()),
        ],
        false,
        false,
    );
    let bytes = writer.bytes.clone();
    let drops = writer.drops.clone();
    let mut file = AppendFile::new(writer, PathBuf::from("script"), 20);
    let request = block(1, 0, b"hello");
    let result = file.append(request.clone());
    assert_eq!(result.prefix().written_bytes(), 2);
    assert_eq!(result.prefix().partial_bytes(), 2);
    assert_eq!(result.prefix().complete_sequence(), 0);
    assert!(
        matches!(result.error(), Some(RecordingError::Append { source, .. }) if source.as_ref().kind() == io::ErrorKind::StorageFull)
    );
    let error = result.error().unwrap();
    let native = native_cause(error);
    assert_eq!(native.kind(), io::ErrorKind::StorageFull);
    let duplicate = file.append(request.clone());
    assert!(std::ptr::eq(
        native,
        native_cause(duplicate.error().unwrap())
    ));
    file.append(request);
    file.append(block(2, 2, b"more"));
    assert_eq!(&*lock(&bytes), b"he");
    let result = file.seal();
    assert!(result.prefix().closed());
    assert!(result.error().is_some());
    file.seal();
    assert_eq!(*lock(&drops), 1);
}
#[test]
fn flush_and_sync_failures_close_once_and_never_claim_sync() {
    for (flush_error, sync_error) in [(true, false), (false, true)] {
        let writer = writer(vec![], flush_error, sync_error);
        let syncs = writer.syncs.clone();
        let drops = writer.drops.clone();
        let mut file = AppendFile::new(writer, PathBuf::from("script"), 20);
        file.append(block(1, 0, b"bytes"));
        let result = file.seal();
        assert_eq!(result.prefix().written_bytes(), 5);
        assert!(result.prefix().closed());
        assert!(!result.prefix().synced());
        let error = result.error().unwrap();
        assert!(matches!(
            (flush_error, error),
            (true, RecordingError::Flush { .. }) | (false, RecordingError::Sync { .. })
        ));
        let native = native_cause(error);
        assert_eq!(
            native.kind(),
            if flush_error {
                io::ErrorKind::BrokenPipe
            } else {
                io::ErrorKind::Other
            }
        );
        let sealed_again = file.seal();
        assert!(std::ptr::eq(
            native,
            native_cause(sealed_again.error().unwrap())
        ));

        assert_eq!(*lock(&syncs), usize::from(!flush_error));
        assert_eq!(*lock(&drops), 1);
    }
}
#[test]
fn incorrect_sequence_poisoning_prevents_uncertain_followup() {
    let writer = writer(vec![], false, false);
    let bytes = writer.bytes.clone();
    let mut file = AppendFile::new(writer, PathBuf::from("script"), 20);
    assert!(matches!(
        file.append(block(2, 0, b"wrong")).error(),
        Some(RecordingError::SequenceMismatch)
    ));
    file.append(block(1, 0, b"later"));
    assert!(lock(&bytes).is_empty());
}

#[test]
fn factory_creates_unique_names_and_invalid_parent_retains_source() {
    let directory = tempfile::tempdir().unwrap();
    let factory = LocalOutputLogFactory::new(directory.path().to_owned());
    let options = RecordingOptions {
        directory: None,
        max_file_mib: 1,
    };
    let first = block_on(factory.create(options.clone())).unwrap();
    let second = block_on(factory.create(options)).unwrap();
    assert_ne!(first.path(), second.path());
    block_on(first.seal());
    block_on(second.seal());
    let parent = directory.path().join("regular-file");
    fs::write(&parent, b"unchanged").unwrap();
    let error = create_at(parent.join("log"), 20).err().unwrap();
    assert!(matches!(error, RecordingError::Create { .. }));
    assert_ne!(native_cause(&error).kind(), io::ErrorKind::Other);
    let clone = error.clone();
    assert!(std::ptr::eq(native_cause(&error), native_cause(&clone)));
    assert_eq!(fs::read(parent).unwrap(), b"unchanged");
}
#[test]
fn short_writes_complete_once_and_zero_write_keeps_partial_prefix() {
    for (steps, expected, failed) in [
        (
            vec![Ok(2), Err(io::ErrorKind::Interrupted.into()), Ok(3)],
            5,
            false,
        ),
        (vec![Ok(2), Ok(0)], 2, true),
    ] {
        let writer = writer(steps, false, false);
        let bytes = writer.bytes.clone();
        let mut file = AppendFile::new(writer, PathBuf::from("script"), 20);
        let request = block(1, 0, b"hello");
        let result = file.append(request.clone());
        assert_eq!(result.prefix().written_bytes(), expected);
        assert_eq!(result.error().is_some(), failed);
        assert_eq!(result.prefix().complete_sequence(), u64::from(!failed));
        file.append(request);
        assert_eq!(lock(&bytes).len() as u64, expected);
    }
}

#[test]
fn supplied_read_only_native_writer_keeps_failure_and_partial_prefix_through_seal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("readonly.log");
    fs::write(&path, b"existing").unwrap();
    let writer = File::open(&path).unwrap();
    let mut file = AppendFile::new(writer, path.clone(), 20);
    let result = file.append(block(1, 0, b"new"));
    let error = result.error().unwrap();
    assert!(
        matches!(error, RecordingError::Append { path: failure_path, offset: 0, .. } if failure_path == &path)
    );
    let native = native_cause(error);
    assert!(native.raw_os_error().is_some());
    assert_eq!(result.prefix().written_bytes(), 0);
    assert_eq!(result.prefix().partial_bytes(), 0);
    let seal = file.seal();
    assert!(seal.prefix().closed());
    assert!(std::ptr::eq(native, native_cause(seal.error().unwrap())));
    assert_eq!(fs::read(&path).unwrap(), b"existing");
}
