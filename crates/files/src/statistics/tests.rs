use super::*;
use nocterm_session::{DirEntry, FsError, FsFuture};
use std::collections::HashMap;

struct TreeFs(HashMap<&'static str, Vec<DirEntry>>);

impl RemoteFs for TreeFs {
    fn home(&self) -> FsFuture<String> {
        Box::pin(async { Ok("/home/u".into()) })
    }
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        let entries = self.0.get(path).cloned();
        Box::pin(async move { entries.ok_or_else(|| FsError::Other("denied".into())) })
    }
}

fn entry(name: &str, kind: EntryKind, size: u64) -> DirEntry {
    DirEntry {
        name: name.into(),
        kind,
        is_symlink: false,
        size: Some(size),
    }
}

fn walk(fs: TreeFs, root: &str, policy: Policy) -> Statistics {
    let progress = Arc::new(Mutex::new(Statistics::default()));
    futures::executor::block_on(scan_remote(
        Arc::new(fs),
        root.into(),
        policy,
        Arc::new(AtomicBool::new(false)),
        progress.clone(),
    ));
    progress.lock().clone()
}

#[test]
fn remote_walk_sizes_nested_files_skips_excluded_and_links() {
    let mut link = entry("loop", EntryKind::Directory, 0);
    link.is_symlink = true;
    let fs = TreeFs(HashMap::from([
        (
            "/srv",
            vec![
                entry(".", EntryKind::Directory, 0),
                entry("a", EntryKind::File, 3),
                entry("app", EntryKind::Directory, 0),
                entry("node_modules", EntryKind::Directory, 0),
                entry("locked", EntryKind::Directory, 0),
                link,
            ],
        ),
        ("/srv/app", vec![entry("b", EntryKind::File, 5)]),
        (
            "/srv/node_modules",
            vec![entry("huge", EntryKind::File, 1 << 30)],
        ),
    ]));
    let statistics = walk(fs, "/srv", Policy::remote(&IndexingSettings::default()));
    assert_eq!(
        (statistics.files, statistics.directories, statistics.bytes),
        (1, 3, 8)
    );
    assert_eq!(statistics.excluded, 1);
    assert_eq!(statistics.inaccessible, 1, "the unreadable folder");
    assert!(statistics.complete);
}

#[test]
fn remote_walk_stops_at_the_entry_limit() {
    let many = (0..30)
        .map(|ix| entry(&format!("f{ix}"), EntryKind::File, 1))
        .collect();
    let mut policy = Policy::remote(&IndexingSettings::default());
    policy.max_entries = 10;
    let statistics = walk(TreeFs(HashMap::from([("/", many)])), "/", policy);
    assert_eq!(statistics.files, 10);
    assert!(statistics.errors[0].contains("10 entry limit"));
    assert!(statistics.complete);
}

#[test]
fn home_root_and_disabled_folders_are_not_counted() {
    let settings = IndexingSettings::default();
    let home = Path::new("/home/u");
    assert_eq!(local_skip(home, Some(home), &settings), Some(Skip::Home));
    assert_eq!(
        local_skip(Path::new("/"), Some(home), &settings),
        Some(Skip::Root)
    );
    assert_eq!(local_skip(&home.join("src"), Some(home), &settings), None);
    assert_eq!(
        remote_skip("/root/", Some("/root"), &settings),
        Some(Skip::Home)
    );
    assert_eq!(remote_skip("/", Some("/root"), &settings), Some(Skip::Root));
    assert_eq!(remote_skip("/etc", Some("/root"), &settings), None);
    let off = IndexingSettings {
        remote: false,
        skip_home: false,
        ..IndexingSettings::default()
    };
    assert_eq!(remote_skip("/etc", None, &off), Some(Skip::Disabled));
    assert_eq!(local_skip(home, Some(home), &off), None);
}

#[test]
fn shallow_statistics_count_only_the_listing_and_say_why() {
    let statistics = shallow(
        [EntryKind::File, EntryKind::Directory, EntryKind::Other].into_iter(),
        Skip::Home,
    );
    assert_eq!((statistics.files, statistics.directories), (1, 1));
    assert_eq!(
        summary(&statistics),
        "1 files · 1 folders · home folder size not counted"
    );
}

#[test]
fn counter_restart_cancels_the_previous_walk() {
    let mut counter = Counter::default();
    let (first, progress) = counter.restart();
    progress.lock().files = 4;
    assert!(counter.poll());
    assert_eq!(counter.shown.files, 4);
    let (second, _) = counter.restart();
    assert!(first.load(Ordering::Acquire));
    assert!(!second.load(Ordering::Acquire));
    assert_eq!(counter.shown, Statistics::default());
    drop(counter);
    assert!(second.load(Ordering::Acquire), "dropping cancels");
}
