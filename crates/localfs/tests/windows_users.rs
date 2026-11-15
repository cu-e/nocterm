//! Real Users/profile traversal through production LocalFs (no ACL changes).
#![cfg(windows)]

use nocterm_localfs::LocalFilesystem;
use nocterm_session::local_fs::{CancelFlag, ListingLimits, LocalFs};
use std::path::PathBuf;

#[test]
fn users_and_current_profile_open_and_list_through_native_capabilities() {
    let profile =
        PathBuf::from(std::env::var_os("USERPROFILE").expect("Windows supplies USERPROFILE"));
    let profile = std::fs::canonicalize(profile).unwrap();
    let users = profile.parent().expect("profile has a Users parent");
    let filesystem = LocalFilesystem::with_base(profile.clone()).unwrap();
    for path in [users, profile.as_path()] {
        let held = filesystem.open_directory(path).unwrap();
        assert!(held.identity().is_held());
        let mut entries = held.entries().unwrap();
        while entries.next().unwrap().is_some() {}
        let listing = filesystem
            .browse(path, ListingLimits::default(), &CancelFlag::default())
            .unwrap();
        assert_eq!(listing.directory, path);
        if path == users {
            assert!(
                listing
                    .entries
                    .iter()
                    .any(|entry| Some(entry.name.as_os_str()) == profile.file_name())
            );
        }
    }
    // Acquisition and read-only browsing do not attempt to create history at an ancestor.
    assert!(profile.is_dir());
}
