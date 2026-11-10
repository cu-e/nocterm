use super::*;
use futures::executor::block_on;
use nocterm_session::{
    FsError, RemoteFs,
    local_fs::LocalFsError,
    testing::{FsOperation, InMemoryFs},
};

#[test]
fn remote_home_and_directory_listing_use_the_captured_source() {
    let source = Arc::new(InMemoryFs::default());
    block_on(source.create_dir("/home")).unwrap();
    source.set_home("/home").unwrap();
    block_on(source.create_dir("/home/資料 folder")).unwrap();
    source.seed_file("/home/file", vec![]).unwrap();
    let reader = RemoteDirectoryReader::new(source);
    assert_eq!(
        block_on(reader.home()).unwrap(),
        Directory::Remote("/home".into())
    );
    let listing =
        block_on(reader.list(Directory::Remote("/home".into()), CancelFlag::default())).unwrap();
    assert_eq!(
        listing
            .candidates
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>(),
        ["資料 folder"]
    );
}
#[test]
fn remote_permission_errors_and_wrong_source_never_fall_back_to_local() {
    let source = Arc::new(InMemoryFs::default());
    source.fail_next(
        FsOperation::List,
        "/private",
        FsError::PermissionDenied {
            path: "/private".into(),
        },
    );
    let reader = RemoteDirectoryReader::new(source);
    let error = block_on(reader.list(Directory::Remote("/private".into()), CancelFlag::default()))
        .unwrap_err();
    assert!(matches!(
        error,
        DirectoryError::Remote(FsError::PermissionDenied { .. })
    ));
    assert!(matches!(
        std::error::Error::source(&error).and_then(|cause| cause.downcast_ref::<FsError>()),
        Some(FsError::PermissionDenied { .. })
    ));
    assert!(matches!(
        block_on(reader.list(Directory::Local("/".into()), CancelFlag::default())),
        Err(DirectoryError::SourceMismatch)
    ));
}
#[test]
fn local_listing_checks_access_and_preserves_original_native_directory() {
    let source = Arc::new(InMemoryFs::default());
    let directory = if cfg!(windows) {
        std::path::PathBuf::from("C:\\workspace")
    } else {
        "/workspace".into()
    };
    source.seed_native_directory(&directory).unwrap();
    source
        .seed_native_directory(&directory.join("資料 folder"))
        .unwrap();
    source
        .seed_native_file(&directory.join("file"), vec![])
        .unwrap();
    let reader = LocalDirectoryReader::new(source);
    let listing =
        block_on(reader.list(Directory::Local(directory.clone()), CancelFlag::default())).unwrap();
    assert_eq!(listing.directory, Directory::Local(directory.clone()));
    assert_eq!(listing.candidates.len(), 1);
    assert_eq!(listing.candidates[0].name, "資料 folder");
    let cancel = CancelFlag::default();
    cancel.cancel();
    assert!(matches!(
        block_on(reader.list(Directory::Local(directory), cancel)),
        Err(DirectoryError::Local(LocalFsError::Cancelled))
    ));
}
#[test]
fn missing_remote_home_retains_its_typed_cause() {
    let source = Arc::new(InMemoryFs::default());
    source.fail_next(FsOperation::Home, "/", FsError::NotConnected);
    let reader = RemoteDirectoryReader::new(source);
    assert!(matches!(
        block_on(reader.home()),
        Err(DirectoryError::Remote(FsError::NotConnected))
    ));
}
