//! Shared publication behavior through the installed service and library ports.
use super::*;
use gpui::TestAppContext;
use nocterm_themes::{Appearance, test_support::InMemoryThemes};
fn info(id: &str) -> ExtensionInfo {
    serde_json::from_str(&format!(r#"{{"id":"{id}","name":"{id}","version":"1"}}"#)).unwrap()
}
fn backend() -> Arc<InMemoryThemes> {
    let backend = Arc::new(InMemoryThemes::default());
    backend.offer(
        info("fake"),
        br#"{"themes":[{"name":"Fresh","appearance":"dark","style":{}}]}"#.to_vec(),
    );
    backend
}
#[gpui::test]
async fn duplicate_admission_is_shared_and_publication_precedes_the_receipt(
    cx: &mut TestAppContext,
) {
    let backend = backend();
    let handle =
        cx.update(|cx| ThemesHandle::install(Some(backend.clone()), None, backend.scan(), cx));
    let admitted = cx.update(|cx| handle.install_pack(info("fake"), cx));
    let duplicate = cx.update(|cx| handle.install_pack(info("fake"), cx));
    assert!(matches!(
        duplicate.await.unwrap_err().as_ref(),
        ThemeOperationError::Busy
    ));
    assert!(cx.update(|cx| handle.busy("fake", cx)));
    admitted.await.unwrap();
    cx.update(|cx| {
        assert!(!handle.busy("fake", cx));
        assert!(handle.catalog(cx).find("Fresh", Appearance::Dark).is_some());
    });
    assert_eq!(backend.installs(), 1);
}
#[gpui::test]
fn dropped_receipt_keeps_the_admitted_mutation_and_its_shared_error(cx: &mut TestAppContext) {
    let backend = backend();
    backend.fail_install("fake", true);
    let handle =
        cx.update(|cx| ThemesHandle::install(Some(backend.clone()), None, backend.scan(), cx));
    drop(cx.update(|cx| handle.install_pack(info("fake"), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!handle.busy("fake", cx));
        assert!(matches!(
            handle.errors(cx).get("fake").unwrap().as_ref(),
            ThemeOperationError::Unsupported
        ));
        assert!(handle.catalog(cx).entries().is_empty());
    });
    assert_eq!(backend.installs(), 1);
}
#[gpui::test]
fn reload_requested_during_a_physically_blocked_install_keeps_committed_catalog(
    cx: &mut TestAppContext,
) {
    let backend = backend();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let wait = release.clone();
    backend.before_install(Some(Arc::new(move || {
        started_tx.send(()).unwrap();
        let (state, changed) = wait.as_ref();
        let mut open = state.lock().unwrap();
        while !*open {
            open = changed.wait(open).unwrap();
        }
    })));
    let handle =
        cx.update(|cx| ThemesHandle::install(Some(backend.clone()), None, backend.scan(), cx));
    drop(cx.update(|cx| handle.install_pack(info("fake"), cx)));
    let dispatcher = cx.executor().dispatcher().clone();
    let worker = std::thread::spawn(move || assert!(dispatcher.as_test().unwrap().tick(true)));
    started_rx.recv().unwrap();
    cx.update(|cx| handle.reload(cx));
    assert!(cx.update(|cx| handle.catalog(cx).entries().is_empty()));
    let (state, changed) = release.as_ref();
    *state.lock().unwrap() = true;
    changed.notify_all();
    worker.join().unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!handle.busy("fake", cx));
        assert!(handle.catalog(cx).find("Fresh", Appearance::Dark).is_some());
    });
}
#[gpui::test]
async fn outstanding_mutation_records_have_bounded_admission(cx: &mut TestAppContext) {
    let backend = backend();
    let handle =
        cx.update(|cx| ThemesHandle::install(Some(backend), None, ThemeCatalog::default(), cx));
    for index in 0..32 {
        drop(cx.update(|cx| handle.install_pack(info(&format!("x{index}")), cx)));
    }
    let refused = cx.update(|cx| handle.install_pack(info("extra"), cx));
    assert!(matches!(
        refused.await.unwrap_err().as_ref(),
        ThemeOperationError::Full
    ));
    cx.run_until_parked();
}
