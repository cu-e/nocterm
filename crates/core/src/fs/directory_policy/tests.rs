use super::*;
use std::error::Error;

#[test]
fn exact_prepared_policy_message_preserves_original_display_kind_and_source() {
    for path in [
        Path::new(""),
        Path::new("relative/名字"),
        Path::new("a/../b//"),
    ] {
        for reason in [
            PrivateDirectoryReason::NotPlain,
            PrivateDirectoryReason::ForeignOwner,
        ] {
            let plan = DirectoryMessagePlan::new(path, reason).unwrap();
            let original = reason.compatibility_error(path);
            let admitted = plan.allocate().unwrap();
            assert_eq!(admitted.kind(), original.kind());
            assert_eq!(admitted.to_string(), original.to_string());
            assert_eq!(format!("{admitted:?}"), format!("{original:?}"));
            assert_eq!(
                admitted.source().map(ToString::to_string),
                original.source().map(ToString::to_string)
            );
            assert!(admitted.source().is_none());
            assert_eq!(
                admitted.get_ref().unwrap().to_string(),
                original.get_ref().unwrap().to_string()
            );
            assert!(admitted.get_ref().unwrap().source().is_none());
        }
    }
}
#[cfg(unix)]
#[test]
fn prepared_policy_message_counts_actual_lossy_native_display_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"native-\xff-\xf0\x80"));
    let reason = PrivateDirectoryReason::NotPlain;
    let plan = DirectoryMessagePlan::new(path, reason).unwrap();
    let expected = reason.compatibility_error(path);
    assert_eq!(plan.allocate().unwrap().to_string(), expected.to_string());
    assert_eq!(plan.capacity, expected.to_string().len());
}
#[test]
fn checked_directory_reports_policy_before_constructing_dynamic_native_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    std::fs::write(&path, b"untouched").unwrap();
    let error = crate::paths::ensure_private_dir_checked(&path).unwrap_err();
    assert!(matches!(
        error,
        PrivateDirectoryCheckError::Policy(PrivateDirectoryReason::NotPlain)
    ));
    let old = crate::paths::ensure_private_dir(&path).unwrap_err();
    assert_eq!(old.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(
        old.to_string(),
        format!("{} is not a plain directory", path.display())
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"untouched");
}
