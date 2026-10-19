#![deny(rust_2018_idioms)]

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(target_os = "linux")]
use std::{
    sync::mpsc::{TryRecvError, sync_channel},
    thread,
};

#[test]
fn test_basic() {
    // For the wasi platforms, `std::env::temp_dir` will panic. For those targets, configure the /tmp
    // directory instead as the base directory for temp files.
    #[cfg(target_os = "wasi")]
    let _ = tempfile::env::override_temp_dir(std::path::Path::new("/tmp"));

    let mut tmpfile = tempfile::tempfile().unwrap();
    write!(tmpfile, "abcde").unwrap();
    tmpfile.seek(SeekFrom::Start(0)).unwrap();
    let mut buf = String::new();
    tmpfile.read_to_string(&mut buf).unwrap();
    assert_eq!("abcde", buf);
}

#[test]
fn test_cleanup() {
    // For the wasi platforms, `std::env::temp_dir` will panic. For those targets, configure the /tmp
    // directory instead as the base directory for temp files.
    #[cfg(target_os = "wasi")]
    let _ = tempfile::env::override_temp_dir(std::path::Path::new("/tmp"));

    let tmpdir = tempfile::tempdir().unwrap();
    {
        let mut tmpfile = tempfile::tempfile_in(&tmpdir).unwrap();
        write!(tmpfile, "abcde").unwrap();
    }
    let num_files = fs::read_dir(&tmpdir).unwrap().count();
    assert!(num_files == 0);
}

// Only run this test on Linux. MacOS doesn't like us creating so many files, apparently.
#[cfg(target_os = "linux")]
#[test]
fn test_pathological_cleaner() {
    let tmpdir = tempfile::tempdir().unwrap();
    let (tx, rx) = sync_channel(0);
    let cleaner_thread = thread::spawn(move || {
        let tmp_path = rx.recv().unwrap();
        while rx.try_recv() == Err(TryRecvError::Empty) {
            let files = fs::read_dir(&tmp_path).unwrap();
            for f in files {
                // skip errors
                if f.is_err() {
                    continue;
                }
                let f = f.unwrap();
                let _ = fs::remove_file(f.path());
            }
        }
    });

    // block until cleaner_thread makes progress
    tx.send(tmpdir.path().to_owned()).unwrap();
    // need 40-400 iterations to encounter race with cleaner on original system
    for _ in 0..10000 {
        let mut tmpfile = tempfile::tempfile_in(&tmpdir).unwrap();
        write!(tmpfile, "abcde").unwrap();
        tmpfile.seek(SeekFrom::Start(0)).unwrap();
        let mut buf = String::new();
        tmpfile.read_to_string(&mut buf).unwrap();
        assert_eq!("abcde", buf);
    }

    // close the channel to make cleaner_thread exit
    drop(tx);
    cleaner_thread.join().expect("The cleaner thread failed");
}

#[cfg(unix)]
#[test]
fn admitted_anonymous_operations_keep_original_native_flags_and_cleanup() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::{
        io,
        path::{Path, PathBuf},
    };
    use tempfile::anonymous::{AnonymousOperations, Name};
    struct Operations {
        conversions: usize,
    }
    impl AnonymousOperations for Operations {
        type Path = PathBuf;
        type Credit = ();
        type Error = io::Error;
        fn current_dir(&mut self) -> io::Result<PathBuf> {
            std::env::current_dir()
        }
        fn join(&mut self, base: &Path, right: &Path) -> io::Result<PathBuf> {
            Ok(base.join(right))
        }
        fn name_buffer(&mut self, capacity: usize) -> io::Result<Name<()>> {
            Name::try_with_capacity(capacity, ())
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
        }
        fn native_path(&mut self, _: &Path) -> io::Result<()> {
            self.conversions += 1;
            Ok(())
        }
        fn native_error(&mut self, error: io::Error) -> io::Error {
            error
        }
        fn error_kind(error: &io::Error) -> Option<io::ErrorKind> {
            Some(error.kind())
        }
        fn exhausted(&mut self, base: &Path) -> io::Error {
            tempfile::anonymous::exhausted_with_owned_path(base.to_owned())
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut operations = Operations { conversions: 0 };
    let mut file = tempfile::anonymous::tempfile_in(directory.path(), &mut operations).unwrap();
    assert!(operations.conversions > 0);
    // The ORIGINAL Linux TMPFILE request uses default0666/umask; only the
    // ORIGINAL named/unlinked fallback applies explicit0600. Observe the same
    // native request independently, instead of guessing a universal mode.
    #[cfg(target_os = "linux")]
    let original_mode = {
        use std::os::unix::fs::OpenOptionsExt;
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(rustix::fs::OFlags::TMPFILE.bits() as i32)
            .open(directory.path())
        {
            Ok(original) => original.metadata().unwrap().permissions().mode() & 0o777,
            Err(error)
                if matches!(
                    rustix::io::Errno::from_io_error(&error),
                    Some(
                        rustix::io::Errno::OPNOTSUPP
                            | rustix::io::Errno::ISDIR
                            | rustix::io::Errno::NOENT
                    )
                ) =>
            {
                0o600
            }
            Err(error) => panic!("original native anonymous request failed: {error}"),
        }
    };
    #[cfg(not(target_os = "linux"))]
    let original_mode = 0o600;
    assert_eq!(
        file.metadata().unwrap().permissions().mode() & 0o777,
        original_mode
    );
    assert_eq!(file.metadata().unwrap().nlink(), 0);
    file.write_all(b"same original anonymous engine").unwrap();
    file.rewind().unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"same original anonymous engine");
    assert_eq!(directory.path().read_dir().unwrap().count(), 0);
    let blocker = directory.path().join("blocker");
    fs::write(&blocker, b"unchanged").unwrap();
    let original = tempfile::tempfile_in(&blocker).unwrap_err();
    let managed = tempfile::anonymous::tempfile_in(&blocker, &mut operations).unwrap_err();
    assert_eq!(managed.kind(), original.kind());
    assert_eq!(managed.raw_os_error(), original.raw_os_error());
    assert_eq!(fs::read(&blocker).unwrap(), b"unchanged");
}
