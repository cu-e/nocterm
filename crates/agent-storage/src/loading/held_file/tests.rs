use super::*;
use serde::Deserialize;
use std::io::Write as _;
struct Number;
impl<'de> DeserializeSeed<'de> for Number {
    type Value = u64;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<u64, D::Error> {
        u64::deserialize(decoder)
    }
}
struct Skip;
impl<'de> DeserializeSeed<'de> for Skip {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        serde::de::IgnoredAny::deserialize(decoder).map(|_| ())
    }
}
fn fixture(bytes: &[u8]) -> (tempfile::TempDir, PathBuf, HeldFile) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("document.json");
    std::fs::write(&path, bytes).unwrap();
    let held = HeldFile::open(&path).unwrap();
    (directory, path, held)
}
#[test]
fn held_file_hashes_each_whole_pass_including_trailing_whitespace() {
    let (_directory, path, mut held) = fixture(b"42 \n\t");
    let stamp = held.preflight().unwrap();
    assert_eq!(stamp.encoded_bytes(), 5);
    assert_eq!(stamp.bounds().nodes, 1);
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 42);
    assert_eq!(std::fs::read(path).unwrap(), b"42 \n\t");
}
#[test]
fn held_file_rejects_changed_inode_bytes_even_at_same_length() {
    let (_directory, path, mut held) = fixture(b"42");
    let stamp = held.preflight().unwrap();
    std::fs::write(&path, b"43").unwrap();
    assert!(matches!(
        held.replay_seed(&stamp, Number),
        Err(HeldReadError::ChangedDocument)
    ));
}
#[test]
fn held_file_reports_changed_document_before_changed_local_syntax() {
    let (_directory, path, mut held) = fixture(b"42");
    let stamp = held.preflight().unwrap();
    std::fs::write(&path, b"xx").unwrap();
    assert!(matches!(
        held.replay_seed(&stamp, Number),
        Err(HeldReadError::ChangedDocument)
    ));
}
#[test]
fn held_receipt_cannot_be_reused_by_another_open_of_the_same_path() {
    let (_directory, path, mut first) = fixture(b"42");
    let stamp = first.preflight().unwrap();
    let mut other = HeldFile::open(&path).unwrap();
    assert!(matches!(
        other.replay_seed(&stamp, Number),
        Err(HeldReadError::ForeignSource)
    ));
}
#[cfg(unix)]
#[test]
fn path_replacement_cannot_retarget_the_held_descriptor() {
    let (directory, path, mut held) = fixture(b"42");
    let stamp = held.preflight().unwrap();
    std::fs::rename(&path, directory.path().join("original")).unwrap();
    std::fs::write(&path, b"99").unwrap();
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 42);
    assert_eq!(std::fs::read(path).unwrap(), b"99");
}
#[cfg(unix)]
#[test]
fn held_open_preserves_existing_final_symlink_following() {
    let (directory, path, _unused) = fixture(b"42");
    let link = directory.path().join("link.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let mut held = HeldFile::open(&link).unwrap();
    let stamp = held.preflight().unwrap();
    std::fs::remove_file(&link).unwrap();
    std::fs::write(&link, b"99").unwrap();
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 42);
}
#[test]
fn second_pass_independently_rejects_growth_past_encoded_limit() {
    let (_directory, path, mut held) = fixture(b"0");
    let stamp = held.preflight().unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    assert!(matches!(
        held.replay_seed(&stamp, Skip),
        Err(HeldReadError::EncodedLimit)
    ));
}
#[test]
fn encoded_limit_is_inclusive_for_both_native_passes() {
    let (_directory, path, _unused) = fixture(b"0");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    let chunk = [b' '; 4096];
    let mut remaining = MAX_FILE_BYTES - 1;
    while remaining > 0 {
        let count = remaining.min(chunk.len() as u64) as usize;
        file.write_all(&chunk[..count]).unwrap();
        remaining -= count as u64;
    }
    drop(file);
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight().unwrap();
    assert_eq!(stamp.encoded_bytes(), MAX_FILE_BYTES);
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 0);
}
#[cfg(unix)]
#[test]
fn nonseekable_source_is_captured_once_and_replayed_without_reopening() {
    use std::os::fd::OwnedFd;
    let (source, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    writer.write_all(b"42").unwrap();
    drop(writer);
    let fd: OwnedFd = source.into();
    let mut held = HeldFile::from_opened(File::from(fd), PathBuf::from("socket")).unwrap();
    let stamp = held.preflight().unwrap();
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 42);
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 42);
}

#[cfg(unix)]
#[test]
fn directory_read_failure_retains_typed_native_cause() {
    let directory = tempfile::tempdir().unwrap();
    let error = match HeldFile::open(directory.path()) {
        Err(error) => error,
        Ok(_) => panic!("directory cannot supply replay bytes"),
    };
    let HeldReadError::Native {
        operation: Operation::Read,
        source,
        ..
    } = error
    else {
        panic!("read error")
    };
    assert!(source.as_ref().raw_os_error().is_some());
}

#[test]
fn authenticated_string_span_replay_uses_the_same_held_source() {
    let (_directory, _path, mut held) = fixture(br#""{\"answer\":42}""#);
    let stamp = held.preflight().unwrap();
    let probe = SpanProbe::default();
    let span = held.replay_spans(&stamp, &probe, probe.seed()).unwrap();
    assert_eq!(span.start(), 0);
    assert_eq!(span.len() as u64, stamp.encoded_bytes());
}
#[test]
fn future_format_is_rejected_after_whole_pass_authentication() {
    let (_directory, _path, mut held) = fixture(br#"{"future":{"kept":[true,null]},"version":9}"#);
    let stamp = held.preflight().unwrap();
    let format = held.replay_seed(&stamp, crate::format::RouteSeed).unwrap();
    assert!(matches!(
        format,
        Err(nocterm_ai::history::HistoryCodecError::UnsupportedVersion(
            9
        ))
    ));
}
#[test]
fn actual_v1_load_remains_read_only_after_held_format_verification() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let id = "689d271a-7214-4d27-8535-d1d48bcb8f4f";
    let bytes = format!(
        r#"{{"version":1,"id":"{id}","agent_id":"provider","updated":1,"entries":[{{"kind":"agent","value":"kept"}}]}}"#
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{id}.json"));
    std::fs::write(&path, &bytes).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight().unwrap();
    assert_eq!(
        held.replay_seed(&stamp, crate::format::RouteSeed)
            .unwrap()
            .unwrap(),
        crate::format::Format::LegacyV1
    );
    let repository =
        crate::FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
    let loaded = futures::executor::block_on(repository.load(id.into())).unwrap();
    assert_eq!(loaded.id(), id);
    assert_eq!(std::fs::read(path).unwrap(), bytes.as_bytes());
}

#[test]
fn anonymous_replay_capture_reuses_encoded_limit_without_source_mutation() {
    let (_directory, path, _unused) = fixture(b"0");
    let source = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    source.set_len(MAX_FILE_BYTES + 1).unwrap();
    assert!(matches!(
        spool::capture(source, &path),
        Err(HeldReadError::EncodedLimit)
    ));
    assert_eq!(std::fs::metadata(path).unwrap().len(), MAX_FILE_BYTES + 1);
}

#[cfg(target_os = "linux")]
#[test]
fn managed_nonseekable_source_replays_original_descriptor_with_paid_anonymous_storage() {
    use nocterm_ai::model::{json::deferred::DecodeLedger, retention::RetentionAuthority};
    use std::{
        net::Shutdown,
        os::{fd::OwnedFd, unix::net::UnixStream},
    };
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = ledger.live_requested_bytes();
    let directory = tempfile::tempdir().unwrap();
    let id = "689d271a-7214-4d27-8535-d1d48bcb8f4f";
    let path =
        crate::writing::native_paths::document(directory.path(), id, ".json", &ledger).unwrap();
    let context = ReadFaultContext::prepare(directory.path(), id, &ledger).unwrap();
    let (mut producer, consumer) = UnixStream::pair().unwrap();
    producer.write_all(b"42 \n\t").unwrap();
    producer.shutdown(Shutdown::Write).unwrap();
    let original: OwnedFd = consumer.into();
    let identity = StrongIdentity::managed(&ledger).unwrap();
    let mut held = HeldFile::from_source(
        File::from(original),
        HeldPath::Managed(path),
        identity,
        Some(context),
        Some(&ledger),
    )
    .unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 42);
    assert_eq!(held.replay_seed(&stamp, Number).unwrap(), 42);
    assert_eq!(directory.path().read_dir().unwrap().count(), 0);
    drop(stamp);
    drop(held);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[cfg(target_os = "linux")]
#[test]
fn managed_nonseekable_admission_failure_drops_original_source_and_all_prospective_credit() {
    use nocterm_ai::model::{json::deferred::DecodeLedger, retention::RetentionAuthority};
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let probe = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = probe.live_requested_bytes();
    drop(probe);
    let ledger = DecodeLedger::new(&authority, baseline).unwrap();
    let (mut producer, consumer) = UnixStream::pair().unwrap();
    producer.write_all(b"42").unwrap();
    let original: OwnedFd = consumer.into();
    // The original reader is consumed even when the first real error-owner
    // admission refuses before tempfile touches its environment or filesystem.
    let error = spool::capture_with_context(
        File::from(original),
        Path::new("public-source"),
        None,
        Some(&ledger),
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        HeldReadError::ManagedReplay(nocterm_ai::storage::AgentStorageError::ManagedAdmission(_))
    ));
    assert_eq!(ledger.live_requested_bytes(), baseline);
    assert!(producer.write_all(b"still original peer").is_err());
}

/// Runs one missing-directory replay case alone in a child process, because
/// the temporary directory and the current directory are process-wide.
#[cfg(target_os = "linux")]
fn missing_replay_directory_case(name: &str, tmpdir: &Path, removed_cwd: Option<&Path>) {
    let test = format!("loading::held_file::tests::{name}");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", &test, "--test-threads=1"])
        .env(MISSING_REPLAY_CHILD, "1")
        .env(DOCUMENT_DIRECTORY, std::env::temp_dir())
        .env("TMPDIR", tmpdir);
    if let Some(directory) = removed_cwd {
        command.env(REMOVED_CWD, directory);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("1 passed"), "{stdout}");
}
#[cfg(target_os = "linux")]
const MISSING_REPLAY_CHILD: &str = "NOCTERM_MISSING_REPLAY_DIRECTORY_CHILD";
#[cfg(target_os = "linux")]
const REMOVED_CWD: &str = "NOCTERM_MISSING_REPLAY_REMOVED_CWD";
#[cfg(target_os = "linux")]
const DOCUMENT_DIRECTORY: &str = "NOCTERM_MISSING_REPLAY_DOCUMENT_DIRECTORY";
#[cfg(target_os = "linux")]
fn enter_removed_current_directory() {
    let removed = std::env::var_os(REMOVED_CWD).unwrap();
    std::fs::create_dir(&removed).unwrap();
    std::env::set_current_dir(&removed).unwrap();
    std::fs::remove_dir(&removed).unwrap();
}

/// The managed replay classifies a missing directory like the unmanaged replay:
/// the saved chat's first native fault, with the original error as its cause.
#[cfg(target_os = "linux")]
fn assert_missing_replay_directory_is_not_found() {
    use nocterm_ai::model::{json::deferred::DecodeLedger, retention::RetentionAuthority};
    use std::{
        net::Shutdown,
        os::{fd::OwnedFd, unix::net::UnixStream},
    };
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = ledger.live_requested_bytes();
    let directory = tempfile::tempdir_in(std::env::var_os(DOCUMENT_DIRECTORY).unwrap()).unwrap();
    let id = "689d271a-7214-4d27-8535-d1d48bcb8f4f";
    let path =
        crate::writing::native_paths::document(directory.path(), id, ".json", &ledger).unwrap();
    let context = ReadFaultContext::prepare(directory.path(), id, &ledger).unwrap();
    let (mut producer, consumer) = UnixStream::pair().unwrap();
    producer.write_all(b"42").unwrap();
    producer.shutdown(Shutdown::Write).unwrap();
    let original: OwnedFd = consumer.into();
    let identity = StrongIdentity::managed(&ledger).unwrap();
    let error = match HeldFile::from_source(
        File::from(original),
        HeldPath::Managed(path),
        identity,
        Some(context),
        Some(&ledger),
    ) {
        Err(error) => error,
        Ok(_) => panic!("replay storage is missing"),
    };
    let HeldReadError::ManagedNative(failure) = &error else {
        panic!("not the held document's native fault: {error:?}")
    };
    assert_eq!(failure.as_ref().kind(), io::ErrorKind::NotFound);
    let mut cause = std::error::Error::source(&error);
    let mut native = None;
    while let Some(error) = cause {
        native = native.or(error.downcast_ref::<io::Error>());
        cause = error.source();
    }
    let native = native.unwrap();
    assert_eq!(native.kind(), io::ErrorKind::NotFound);
    // No wrapper adds its own prefix between the document and the native cause.
    assert!(
        error.to_string().ends_with(&format!(": {native}")),
        "{error}"
    );
    drop(error);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[cfg(target_os = "linux")]
#[test]
fn managed_replay_in_a_missing_temp_directory_reports_the_saved_chat_as_not_found() {
    if std::env::var_os(MISSING_REPLAY_CHILD).is_some() {
        return assert_missing_replay_directory_is_not_found();
    }
    let parent = tempfile::tempdir().unwrap();
    missing_replay_directory_case(
        "managed_replay_in_a_missing_temp_directory_reports_the_saved_chat_as_not_found",
        &parent.path().join("absent"),
        None,
    );
}

#[cfg(target_os = "linux")]
#[test]
fn managed_replay_from_a_removed_current_directory_reports_the_saved_chat_as_not_found() {
    if std::env::var_os(MISSING_REPLAY_CHILD).is_some() {
        enter_removed_current_directory();
        return assert_missing_replay_directory_is_not_found();
    }
    let parent = tempfile::tempdir().unwrap();
    missing_replay_directory_case(
        "managed_replay_from_a_removed_current_directory_reports_the_saved_chat_as_not_found",
        Path::new("relative-replay"),
        Some(&parent.path().join("removed")),
    );
}

/// Without a held fault context the replay's own failure still displays and
/// sources the original native error, without the current-directory prefix.
#[cfg(target_os = "linux")]
#[test]
fn managed_replay_failure_without_a_fault_context_keeps_the_native_cause_unwrapped() {
    use nocterm_ai::model::{json::deferred::DecodeLedger, retention::RetentionAuthority};
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    if std::env::var_os(MISSING_REPLAY_CHILD).is_none() {
        let parent = tempfile::tempdir().unwrap();
        return missing_replay_directory_case(
            "managed_replay_failure_without_a_fault_context_keeps_the_native_cause_unwrapped",
            Path::new("relative-replay"),
            Some(&parent.path().join("removed")),
        );
    }
    enter_removed_current_directory();
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = ledger.live_requested_bytes();
    let (_producer, consumer) = UnixStream::pair().unwrap();
    let original: OwnedFd = consumer.into();
    let error = spool::capture_with_context(
        File::from(original),
        Path::new("public-source"),
        None,
        Some(&ledger),
    )
    .err()
    .unwrap();
    let HeldReadError::ManagedReplay(nocterm_ai::storage::AgentStorageError::ManagedReplayRead(
        failure,
    )) = &error
    else {
        panic!("not the replay's own failure: {error:?}")
    };
    let native = std::error::Error::source(failure)
        .and_then(|failure| failure.source())
        .and_then(|native| native.downcast_ref::<io::Error>())
        .unwrap();
    assert_eq!(native.kind(), io::ErrorKind::NotFound);
    assert_eq!(
        error.to_string(),
        format!("Could not read public-source: {native}")
    );
    drop(error);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
