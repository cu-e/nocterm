use nocterm_localfs::LocalFilesystem;
use nocterm_session::local_fs::{CancelFlag, ListingLimits, LocalFs, LocalFsError};
use std::{ffi::OsStr, path::Path};
#[cfg(unix)]
use std::{io::Write, time::Duration};

#[test]
fn listing_and_sources_preserve_names_bounds_and_same_opened_metadata() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("z"), b"abc").unwrap();
    std::fs::write(root.path().join("a"), b"x").unwrap();
    std::fs::create_dir(root.path().join("B")).unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let listing = fs
        .browse(
            Path::new("."),
            ListingLimits::default(),
            &CancelFlag::default(),
        )
        .unwrap();
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| entry.name.to_string_lossy())
            .collect::<Vec<_>>(),
        ["B", "a", "z"]
    );
    assert!(
        fs.browse(
            root.path(),
            ListingLimits::new(2, 100).unwrap(),
            &CancelFlag::default()
        )
        .is_err()
    );
    let mut source = fs.open_source(Path::new("z")).unwrap();
    std::fs::rename(root.path().join("z"), root.path().join("held")).unwrap();
    std::fs::write(root.path().join("z"), b"external").unwrap();
    assert_eq!(source.metadata().unwrap().size, Some(3));
    assert_eq!(source.read(2).unwrap(), b"ab");
    assert_eq!(source.read(2).unwrap(), b"c");
    assert!(source.read(2).unwrap().is_empty());
    source.close().unwrap();
    assert!(fs.resolve_source(Path::new("../outside")).is_err());
    let cancel = CancelFlag::default();
    cancel.cancel();
    assert!(matches!(
        fs.browse(root.path(), ListingLimits::default(), &cancel),
        Err(LocalFsError::Cancelled)
    ));
}

#[cfg(unix)]
#[test]
fn browsing_links_is_read_only_and_strict_handles_survive_ancestor_swaps() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let selected = root.path().join("selected");
    std::fs::create_dir(&selected).unwrap();
    std::fs::write(selected.join("original"), b"keep").unwrap();
    std::fs::write(outside.path().join("external"), b"secret").unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("alias")).unwrap();
    let listing = fs
        .browse(
            &root.path().join("alias"),
            ListingLimits::default(),
            &CancelFlag::default(),
        )
        .unwrap();
    assert_eq!(listing.entries[0].name, "external");
    let listing = fs
        .browse(
            root.path(),
            ListingLimits::default(),
            &CancelFlag::default(),
        )
        .unwrap();
    assert!(
        listing
            .entries
            .iter()
            .find(|entry| entry.name == "alias")
            .unwrap()
            .completion_directory
    );
    assert!(fs.open_directory(&root.path().join("alias")).is_err());
    assert!(fs.open_source(&root.path().join("alias/external")).is_err());
    let held = fs.open_directory(&selected).unwrap();
    let identity = held.identity();
    std::fs::rename(&selected, root.path().join("moved")).unwrap();
    std::os::unix::fs::symlink(outside.path(), &selected).unwrap();
    assert_eq!(
        identity,
        fs.open_directory(&root.path().join("moved"))
            .unwrap()
            .identity()
    );
    let mut cursor = held.entries().unwrap();
    assert_eq!(cursor.next().unwrap().unwrap(), "original");
    assert!(cursor.next().unwrap().is_none());
    assert_eq!(
        held.metadata(OsStr::new("original")).unwrap().unwrap().size,
        Some(4)
    );
    let mut source = held.open_source(OsStr::new("original")).unwrap();
    assert_eq!(
        source.read(32).unwrap(),
        b"keep",
        "Held directory source never reopens the replaced ancestor"
    );
    assert_eq!(source.metadata().unwrap().size, Some(4));
    source.close().unwrap();
    assert!(held.open_source(OsStr::new("../external")).is_err());
    let child = held.create_child(OsStr::new("child")).unwrap();
    assert!(root.path().join("moved/child").is_dir());
    assert!(!outside.path().join("child").exists());
    drop(child);
}

#[cfg(unix)]
#[test]
fn fifo_replacement_is_rejected_without_waiting_for_a_writer() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("selected");
    std::fs::write(&source, b"regular").unwrap();
    assert!(std::fs::metadata(&source).unwrap().is_file());
    std::fs::remove_file(&source).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &source,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let worker_source = source.clone();
    let worker = std::thread::spawn(move || {
        let _ = send.send(fs.open_source(&worker_source).map(|_| ()));
    });
    let result = receive.recv_timeout(Duration::from_secs(1));
    if result.is_err() {
        // Rescue an accidentally blocking implementation before failing.
        let rescue = rustix::fs::open(
            &source,
            rustix::fs::OFlags::WRONLY | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        );
        if let Ok(fd) = rescue {
            let mut file = std::fs::File::from(fd);
            let _ = file.write_all(b"rescue");
            worker.join().unwrap();
        }
        panic!("opening the replaced FIFO blocked waiting for a writer");
    }
    assert!(result.unwrap().is_err());
    worker.join().unwrap();
}
