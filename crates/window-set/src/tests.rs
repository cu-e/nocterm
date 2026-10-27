use std::sync::Mutex;

use gpui::{AnyWindowHandle, Context, IntoElement, Render, TestAppContext, Window};
use nocterm_hubs_model::{HubAddress, HubId};
use nocterm_store::StoreError;

use super::*;

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

fn hub(address: &str) -> Environment {
    Environment::Hub(HubId::from(HubAddress::parse(address).unwrap()))
}

type Saved = Arc<Mutex<Vec<WindowDocument>>>;

/// A set that remembers every document it was asked to write.
fn installed(
    prepared: PreparedWindows,
    shutdown: &ShutdownCoordinator,
    cx: &mut TestAppContext,
) -> (WindowSet, Saved) {
    let saved = Saved::default();
    let log = saved.clone();
    let writer: SnapshotWriter<WindowDocument> = Arc::new(move |document| {
        log.lock().unwrap().push(document);
        Box::pin(async { Ok(()) })
    });
    let set = cx
        .update(|cx| WindowSet::install(prepared, writer, shutdown, cx))
        .unwrap();
    (set, saved)
}

fn open(set: &WindowSet, environment: Environment, cx: &mut TestAppContext) -> AnyWindowHandle {
    let handle = AnyWindowHandle::from(cx.add_window(|_, _| Blank));
    cx.update(|cx| set.opened(handle.window_id(), environment, cx));
    handle
}

fn close(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update(|cx| cx.update_window(window, |_, window, _| window.remove_window()))
        .unwrap();
    cx.run_until_parked();
}

fn environments(set: &WindowSet, cx: &mut TestAppContext) -> Vec<Environment> {
    cx.update(|cx| set.environments(cx))
}

fn last_saved(saved: &Saved) -> Vec<Environment> {
    let saved = saved.lock().unwrap();
    saved
        .last()
        .map(|document| document.environments().cloned().collect())
        .unwrap_or_default()
}

fn quit(shutdown: &ShutdownCoordinator, cx: &mut TestAppContext) {
    let work = cx.update(|cx| shutdown.capture_all(cx));
    cx.background_executor.spawn(work).detach();
    cx.run_until_parked();
}

fn document(environments: &[Environment]) -> WindowDocument {
    let mut windows = OpenWindows::default();
    for (key, environment) in environments.iter().enumerate() {
        windows.opened(key, environment.clone());
    }
    WindowDocument::decode(&windows.document().encode().unwrap()).unwrap()
}

#[gpui::test]
fn a_window_the_user_closes_is_forgotten_and_the_rest_are_saved(cx: &mut TestAppContext) {
    let (set, saved) = installed(
        PreparedWindows::memory(),
        &ShutdownCoordinator::default(),
        cx,
    );
    let first = open(&set, Environment::Local, cx);
    open(&set, hub("hub.home"), cx);
    open(&set, Environment::Local, cx);

    close(first, cx);

    assert_eq!(
        environments(&set, cx),
        [hub("hub.home:7443"), Environment::Local]
    );
    assert_eq!(last_saved(&saved), environments(&set, cx));
}

#[gpui::test]
fn the_last_window_stays_when_it_closes(cx: &mut TestAppContext) {
    let (set, saved) = installed(
        PreparedWindows::memory(),
        &ShutdownCoordinator::default(),
        cx,
    );
    let local = open(&set, Environment::Local, cx);
    let remote = open(&set, hub("hub.home"), cx);

    close(local, cx);
    close(remote, cx);

    assert_eq!(environments(&set, cx), [hub("hub.home:7443")]);
    assert_eq!(last_saved(&saved), [hub("hub.home:7443")]);
}

#[gpui::test]
fn quitting_saves_every_open_window_in_order(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let (set, saved) = installed(PreparedWindows::memory(), &shutdown, cx);
    open(&set, hub("hub.home"), cx);
    open(&set, Environment::Local, cx);
    open(&set, hub("other.example:9000"), cx);

    quit(&shutdown, cx);

    assert_eq!(
        last_saved(&saved),
        [
            hub("hub.home:7443"),
            Environment::Local,
            hub("other.example:9000")
        ]
    );
}

#[gpui::test]
fn closing_windows_before_quitting_leaves_only_the_remaining_ones_saved(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let (set, saved) = installed(PreparedWindows::memory(), &shutdown, cx);
    let first = open(&set, Environment::Local, cx);
    open(&set, hub("hub.home"), cx);

    close(first, cx);
    quit(&shutdown, cx);

    assert_eq!(last_saved(&saved), [hub("hub.home:7443")]);
}

#[gpui::test]
fn switching_a_window_changes_what_is_saved_but_not_its_place(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let (set, saved) = installed(PreparedWindows::memory(), &shutdown, cx);
    let first = open(&set, Environment::Local, cx);
    open(&set, Environment::Local, cx);

    cx.update(|cx| set.switched(first.window_id(), hub("hub.home"), cx));
    quit(&shutdown, cx);

    assert_eq!(
        last_saved(&saved),
        [hub("hub.home:7443"), Environment::Local]
    );
}

#[gpui::test]
fn nothing_is_written_until_a_window_opens(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let prepared = PreparedWindows::loaded(Ok(Some(document(&[hub("hub.home")]))));
    let (_set, saved) = installed(prepared, &shutdown, cx);

    quit(&shutdown, cx);

    assert!(saved.lock().unwrap().is_empty());
}

#[gpui::test]
fn the_previous_run_is_what_the_document_listed(cx: &mut TestAppContext) {
    let listed = [Environment::Local, hub("hub.home"), Environment::Local];
    let prepared = PreparedWindows::loaded(Ok(Some(document(&listed))));
    let (set, _) = installed(prepared, &ShutdownCoordinator::default(), cx);

    assert_eq!(
        set.previous(),
        [Environment::Local, hub("hub.home:7443"), Environment::Local]
    );
    assert!(environments(&set, cx).is_empty());
}

#[gpui::test]
fn a_missing_or_unreadable_document_leaves_nothing_to_restore(cx: &mut TestAppContext) {
    let unreadable = WindowDocument::decode("garbage").unwrap_err();
    for prepared in [
        PreparedWindows::memory(),
        PreparedWindows::loaded(Err(unreadable)),
    ] {
        let (set, _) = installed(prepared, &ShutdownCoordinator::default(), cx);
        assert!(set.previous().is_empty());
    }
}

#[gpui::test]
fn a_set_installed_while_shutting_down_is_refused(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    quit(&shutdown, cx);
    let writer: SnapshotWriter<WindowDocument> = Arc::new(|_| Box::pin(async { Ok(()) }));

    let error = cx
        .update(|cx| WindowSet::install(PreparedWindows::memory(), writer, &shutdown, cx))
        .err()
        .unwrap();

    assert!(matches!(
        error,
        WindowSetError::Registration(StoreError::Closing)
    ));
    assert!(std::error::Error::source(&error).is_some());
}

#[gpui::test]
fn a_kept_environment_is_saved_with_the_windows_that_opened(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let (set, saved) = installed(PreparedWindows::memory(), &shutdown, cx);

    cx.update(|cx| set.kept(hub("away.example"), cx));
    open(&set, Environment::Local, cx);
    quit(&shutdown, cx);

    assert_eq!(
        last_saved(&saved),
        [hub("away.example:7443"), Environment::Local]
    );
    assert_eq!(environments(&set, cx).len(), 2);
}

#[gpui::test]
fn a_document_of_a_newer_version_is_never_written_over(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let newer = WindowDocument::decode(r#"{"version":99,"windows":[]}"#).unwrap_err();
    let (set, saved) = installed(PreparedWindows::loaded(Err(newer)), &shutdown, cx);
    assert!(set.previous().is_empty());

    open(&set, Environment::Local, cx);
    quit(&shutdown, cx);

    assert!(saved.lock().unwrap().is_empty());
    assert_eq!(environments(&set, cx), [Environment::Local]);
}

#[gpui::test]
fn an_unreadable_document_is_replaced_by_the_first_save(cx: &mut TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let unreadable = WindowDocument::decode("garbage").unwrap_err();
    let (set, saved) = installed(PreparedWindows::loaded(Err(unreadable)), &shutdown, cx);

    open(&set, Environment::Local, cx);
    quit(&shutdown, cx);

    assert_eq!(last_saved(&saved), [Environment::Local]);
}
