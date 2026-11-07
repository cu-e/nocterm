//! Native preparation is fenced by the real announcement port and captured source lease.
use super::*;
use futures::FutureExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use nocterm_connections_store::{ProbeSourceIdentity, test_support::ScriptedProbeSource};
use nocterm_session::testing::{Gate, InMemoryFs};
use nocterm_ssh_keys::{authorized_keys::HostIdentity, testing::ScriptedConnectedHosts};
use std::time::{Duration, Instant};

fn host(connection: &str, fs: Arc<dyn RemoteFs>) -> ConnectedHost {
    ConnectedHost {
        connection: connection.into(),
        identity: HostIdentity {
            user: "user".into(),
            host: "host".into(),
            host_key: String::new(),
        },
        port: 22,
        fs,
    }
}
fn source(cx: &mut TestAppContext) -> ScriptedProbeSource {
    let identity = cx.update(|cx| cx.new(|_| ()).entity_id());
    ScriptedProbeSource::new(ProbeSourceIdentity::new(identity, 1))
}
fn observer(cx: &mut TestAppContext, native: &ScriptedConnectedHosts) -> Announcing {
    cx.executor().allow_parking();
    Announcing::new(Arc::new(native.clone()), cx.executor().clone())
}
fn start_preparation(
    cx: &TestAppContext,
    native: &ScriptedConnectedHosts,
) -> std::thread::JoinHandle<()> {
    let dispatcher = cx.executor().dispatcher().clone();
    let worker = std::thread::spawn(move || assert!(dispatcher.as_test().unwrap().tick(true)));
    let deadline = Instant::now() + Duration::from_secs(5);
    // Do not poll the foreground completion waiter until this native worker
    // joins; deterministic local scheduling belongs to the test thread.
    while native.prepared() == 0 {
        assert!(
            Instant::now() < deadline,
            "native preparation did not start"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    worker
}

#[gpui_kit::test]
fn newer_session_announcements_cannot_be_overwritten_by_old_preparation(cx: &mut TestAppContext) {
    let native = ScriptedConnectedHosts::default();
    let gate = Gate::pending();
    let waiting = gate.clone();
    native.suspend_prepare(async move { waiting.wait().await }.boxed());
    let observer = observer(cx, &native);
    let old = source(cx);
    let latest = source(cx);
    let first: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    let replacement: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    cx.update(|cx| observer.connected(host("saved", first), old.lease(), cx));
    let worker = start_preparation(cx, &native);
    cx.update(|cx| observer.connected(host("saved", replacement.clone()), latest.lease(), cx));
    gate.release();
    worker.join().unwrap();
    cx.run_until_parked();
    assert_eq!(native.published().len(), 1);
    assert!(Arc::ptr_eq(&native.published()[0].fs, &replacement));
    assert_eq!(native.prepared(), 2);
}

#[gpui_kit::test]
fn a_closed_original_session_discards_its_prepared_metadata(cx: &mut TestAppContext) {
    let native = ScriptedConnectedHosts::default();
    let gate = Gate::pending();
    let waiting = gate.clone();
    native.suspend_prepare(async move { waiting.wait().await }.boxed());
    let observer = observer(cx, &native);
    let original = source(cx);
    let other = source(cx);
    cx.update(|cx| {
        observer.connected(
            host("closed", Arc::new(InMemoryFs::default())),
            original.lease(),
            cx,
        )
    });
    let worker = start_preparation(cx, &native);
    original.set_current(false);
    cx.update(|cx| {
        observer.connected(
            host("other", Arc::new(InMemoryFs::default())),
            other.lease(),
            cx,
        )
    });
    gate.release();
    worker.join().unwrap();
    cx.run_until_parked();
    assert_eq!(native.published().len(), 1);
    assert_eq!(native.published()[0].connection, "other");
}

#[gpui_kit::test]
fn repeated_announcements_of_the_same_source_share_native_preparation(cx: &mut TestAppContext) {
    let native = ScriptedConnectedHosts::default();
    let gate = Gate::pending();
    let waiting = gate.clone();
    native.suspend_prepare(async move { waiting.wait().await }.boxed());
    let observer = observer(cx, &native);
    let original = source(cx);
    let lease = original.lease();
    let fs: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    cx.update(|cx| observer.connected(host("saved", fs.clone()), lease.clone(), cx));
    let worker = start_preparation(cx, &native);
    for _ in 0..100 {
        cx.update(|cx| observer.connected(host("saved", fs.clone()), lease.clone(), cx));
    }
    let barrier = source(cx);
    cx.update(|cx| {
        observer.connected(
            host("barrier", Arc::new(InMemoryFs::default())),
            barrier.lease(),
            cx,
        )
    });
    gate.release();
    worker.join().unwrap();
    cx.run_until_parked();
    assert_eq!(native.published().len(), 2);
    assert_eq!(native.prepared(), 2);
}
