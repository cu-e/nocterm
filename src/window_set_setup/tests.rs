use super::*;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use nocterm_hubs_model::{Environment, HubAddress, HubId, OpenWindows};
use nocterm_store::ShutdownCoordinator;
use nocterm_window_set::WindowSet;

fn hub() -> Environment {
    Environment::Hub(HubId::from(HubAddress::parse("hub.home").unwrap()))
}

fn paths() -> (tempfile::TempDir, Paths) {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(directory.path());
    std::fs::create_dir_all(paths.state_dir()).unwrap();
    (directory, paths)
}

fn write(paths: &Paths, text: &str) {
    std::fs::write(file_of(paths), text).unwrap();
}

fn previous(paths: &Paths, cx: &mut TestAppContext) -> Vec<Environment> {
    let (prepared, writer) = native(paths);
    cx.update(|cx| {
        WindowSet::install(prepared, writer, &ShutdownCoordinator::default(), cx)
            .unwrap()
            .previous()
            .iter()
            .map(|record| record.environment().clone())
            .collect()
    })
}

#[gpui_kit::test]
fn the_windows_of_the_previous_run_are_read_from_the_state_directory(cx: &mut TestAppContext) {
    let (_directory, paths) = paths();
    let mut windows = OpenWindows::default();
    windows.opened(1, Environment::Local);
    windows.opened(2, hub());
    write(&paths, &windows.document().encode().unwrap());

    assert_eq!(previous(&paths, cx), [Environment::Local, hub()]);
}

#[gpui_kit::test]
fn no_file_means_nothing_to_restore(cx: &mut TestAppContext) {
    let (_directory, paths) = paths();

    assert!(previous(&paths, cx).is_empty());
}

#[gpui_kit::test]
fn a_corrupt_file_means_nothing_to_restore(cx: &mut TestAppContext) {
    let (_directory, paths) = paths();
    write(&paths, "{ this is not a document");

    assert!(previous(&paths, cx).is_empty());
}

#[gpui_kit::test]
fn quitting_writes_the_open_windows_where_the_next_start_reads_them(cx: &mut TestAppContext) {
    let (_directory, paths) = paths();
    let shutdown = ShutdownCoordinator::default();
    let (prepared, writer) = native(&paths);
    cx.update(|cx| {
        let set = WindowSet::install(prepared, writer, &shutdown, cx).unwrap();
        for environment in [Environment::Local, hub()] {
            let window = cx
                .open_window(Default::default(), |_, cx| cx.new(|_| gpui_kit::Empty))
                .unwrap();
            set.opened(AnyWindowHandle::from(window).window_id(), environment, cx);
        }
    });
    let work = cx.update(|cx| shutdown.capture_all(cx));
    cx.background_executor.spawn(work).detach();
    cx.run_until_parked();

    assert_eq!(previous(&paths, cx), [Environment::Local, hub()]);
}

#[gpui_kit::test]
fn a_file_written_by_a_newer_version_is_left_as_it_was(cx: &mut TestAppContext) {
    let (_directory, paths) = paths();
    let newer = r#"{"version":99,"windows":[{"environment":{"kind":"orbit"}}]}"#;
    write(&paths, newer);
    let shutdown = ShutdownCoordinator::default();
    let (prepared, writer) = native(&paths);
    cx.update(|cx| {
        let set = WindowSet::install(prepared, writer, &shutdown, cx).unwrap();
        assert!(set.previous().is_empty());
        let window = cx
            .open_window(Default::default(), |_, cx| cx.new(|_| gpui_kit::Empty))
            .unwrap();
        set.opened(AnyWindowHandle::from(window).window_id(), hub(), cx);
    });
    let work = cx.update(|cx| shutdown.capture_all(cx));
    cx.background_executor.spawn(work).detach();
    cx.run_until_parked();

    assert_eq!(std::fs::read_to_string(file_of(&paths)).unwrap(), newer);
}

#[gpui_kit::test]
fn one_record_that_cannot_be_read_does_not_lose_the_others(cx: &mut TestAppContext) {
    let (_directory, paths) = paths();
    write(
        &paths,
        r#"{"version":1,"windows":[{"environment":{"kind":"orbit"}},{"environment":{"kind":"local"}}]}"#,
    );

    assert_eq!(previous(&paths, cx), [Environment::Local]);
}
