use super::*;
use crate::{EntryKind, FileMetadata};
use std::ffi::OsStr;
fn metadata(kind: EntryKind, size: Option<u64>, is_symlink: bool) -> FileMetadata {
    FileMetadata {
        kind,
        size,
        is_symlink,
        permissions: None,
        uid: None,
        gid: None,
        modified: None,
    }
}

#[test]
fn policy_counts_immediate_children_and_nested_bytes_without_entering_links_or_exclusions() {
    let policy = ScanPolicy::new(vec!["ignored".into()], 100, 256).unwrap();
    let mut count = StatisticsAccumulator::new(policy);
    assert_eq!(
        count.observe(
            &metadata(EntryKind::Directory, None, false),
            1,
            true,
            OsStr::new("ignored")
        ),
        ScanDecision::Skip
    );
    assert_eq!(
        count.observe(
            &metadata(EntryKind::Directory, None, false),
            1,
            true,
            OsStr::new("nested")
        ),
        ScanDecision::Enter
    );
    count.observe(
        &metadata(EntryKind::File, Some(3), false),
        1,
        true,
        OsStr::new("one"),
    );
    count.observe(
        &metadata(EntryKind::File, Some(5), false),
        2,
        false,
        OsStr::new("two"),
    );
    count.observe(
        &metadata(EntryKind::Directory, None, true),
        1,
        true,
        OsStr::new("link"),
    );
    let result = count.finish();
    assert_eq!(
        (
            result.files,
            result.directories,
            result.bytes,
            result.excluded
        ),
        (1, 2, 8, 1)
    );
    assert!(result.complete);
}

#[test]
fn depth_entry_and_error_detail_budgets_are_independent_and_cancellation_stays_incomplete() {
    let mut count = StatisticsAccumulator::new(ScanPolicy::new(vec![], 2, 1).unwrap());
    assert_eq!(
        count.observe(
            &metadata(EntryKind::Directory, None, false),
            1,
            true,
            OsStr::new("deep")
        ),
        ScanDecision::Skip
    );
    assert_eq!(
        count.observe(
            &metadata(EntryKind::File, Some(u64::MAX), false),
            1,
            true,
            OsStr::new("huge")
        ),
        ScanDecision::Skip
    );
    assert_eq!(
        count.observe(
            &metadata(EntryKind::File, Some(1), false),
            1,
            true,
            OsStr::new("excess")
        ),
        ScanDecision::Stop
    );
    for _ in 0..100 {
        count.record_issue(ScanIssue::Local(LocalFsError::Cancelled));
    }
    let result = count.cancelled();
    assert!(!result.complete);
    assert_eq!(result.bytes, u64::MAX);
    assert_eq!(result.inaccessible, 102);
    assert_eq!(result.errors.len(), 12);
}

#[test]
fn native_name_validation_keeps_platform_rules_separate_from_remote_posix_names() {
    for name in ["", ".", "..", "a/b", "a\0b"] {
        assert!(!valid_native_name(OsStr::new(name)));
    }
    #[cfg(unix)]
    {
        assert!(valid_native_name(OsStr::new("colon:stream")));
        assert!(valid_native_name(OsStr::new("back\\slash")));
    }
    #[cfg(windows)]
    for name in [
        "C:new.txt",
        "source:stream",
        "a\\b",
        "trailing.",
        "trailing ",
        "CON",
        "NUL.txt",
        "com1.log",
        "LPT9",
    ] {
        assert!(!valid_native_name(OsStr::new(name)), "{name}");
    }
}

#[test]
fn caller_supplied_walk_limits_cannot_remove_production_bounds_and_skip_policy_uses_explicit_context()
 {
    use std::path::Path;
    assert!(ListingLimits::new(100_001, 8 * 1024 * 1024).is_err());
    assert!(ListingLimits::new(100_000, 8 * 1024 * 1024 + 1).is_err());
    assert!(RemovalLimits::new(257, 1_000_000).is_err());
    assert!(ScanPolicy::new(vec![], 1_000_001, 256).is_err());
    assert!(ScanPolicy::new(vec![], 1_000_000, 257).is_err());
    assert_eq!(
        local_skip(
            Path::new("/home/test"),
            Some(Path::new("/home/test")),
            true,
            true,
            true
        ),
        Some(ScanSkip::Home)
    );
    assert_eq!(
        remote_skip("/home/test/", Some("/home/test"), true, true, false),
        Some(ScanSkip::Home)
    );
    assert_eq!(
        remote_skip("/", None, true, false, true),
        Some(ScanSkip::Root)
    );
    assert_eq!(
        remote_skip("/", None, false, false, true),
        Some(ScanSkip::Disabled)
    );
    let count = Statistics::shallow(
        [EntryKind::File, EntryKind::Directory, EntryKind::Other].into_iter(),
        ScanSkip::Disabled,
    );
    assert!(count.complete);
    assert_eq!(
        (count.files, count.directories, count.bytes, count.skipped),
        (1, 1, 0, Some(ScanSkip::Disabled))
    );
}

#[test]
fn skipped_links_still_consume_the_walk_entry_budget() {
    let mut count = StatisticsAccumulator::new(ScanPolicy::new(vec![], 1, 256).unwrap());
    let link = metadata(EntryKind::Directory, None, true);
    assert_eq!(
        count.observe(&link, 1, true, OsStr::new("one")),
        ScanDecision::Skip
    );
    assert_eq!(
        count.observe(&link, 1, true, OsStr::new("two")),
        ScanDecision::Stop
    );
    let result = count.finish();
    assert_eq!((result.files, result.directories, result.bytes), (0, 0, 0));
    assert_eq!(result.inaccessible, 1);
}

#[test]
fn unreadable_children_consume_the_same_budget_as_successful_metadata() {
    let mut count = StatisticsAccumulator::new(ScanPolicy::new(vec![], 1, 256).unwrap());
    let missing = std::path::PathBuf::from("/missing");
    assert_eq!(
        count.observe_entry_failure(missing.clone(), LocalFsError::NotFound(missing)),
        ScanDecision::Skip
    );
    assert_eq!(
        count.observe(
            &metadata(EntryKind::File, Some(100), false),
            1,
            true,
            OsStr::new("excess")
        ),
        ScanDecision::Stop
    );
    let result = count.finish();
    assert_eq!(result.bytes, 0);
    assert_eq!(result.inaccessible, 2);
    assert!(matches!(
        result.errors.last(),
        Some(ScanIssue::Local(LocalFsError::BoundExceeded(
            "statistics entries"
        )))
    ));
}
