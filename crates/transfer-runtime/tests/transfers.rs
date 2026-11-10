//! Public transfer-service behavior over shared filesystem port fakes.
mod support;
use support::*;
#[path = "cases/downloads.rs"]
mod downloads;
#[path = "cases/relays.rs"]
mod relays;
#[path = "cases/uploads.rs"]
mod uploads;

#[test]
fn another_transfer_finishes_while_an_earlier_remote_write_is_suspended() {
    use nocterm_session::{
        HostAddress, RemoteFs as _, Target,
        testing::{FsOperation, Gate, InMemoryFs},
    };
    use nocterm_transfers::{
        CollisionPolicy, TransferRequests as _, TransferService as _, UploadRequest,
    };
    use std::sync::Arc;
    let fixture = Fixture::new();
    fixture
        .local
        .seed_native_file(&native("input/slow"), b"slow".to_vec())
        .unwrap();
    fixture
        .local
        .seed_native_file(&native("input/fast"), b"fast".to_vec())
        .unwrap();
    let gate = Gate::pending();
    fixture
        .remote
        .suspend_next(FsOperation::Write, "/dest/slow", gate.clone());
    let slow = fixture.upload(vec![native("input/slow")], CollisionPolicy::Rename);
    wait(|| fixture.remote.activity().live_uploads > 0);
    let elsewhere = Arc::new(InMemoryFs::default());
    futures::executor::block_on(elsewhere.create_dir("/dest")).unwrap();
    let fast = fixture
        .runtime
        .enqueue(UploadRequest {
            sources: vec![native("input/fast")],
            host: HostAddress::direct(Target::new("u", "another-host", 22)),
            destination: "/dest".into(),
            fs: elsewhere.clone(),
            collisions: CollisionPolicy::Rename,
        })
        .unwrap();
    assert_eq!(fixture.finished(fast).completed_files, 1);
    assert!(!fixture.runtime.progress(slow).unwrap().state.finished());
    assert_eq!(elsewhere.bytes("/dest/fast").unwrap(), b"fast");
    gate.release();
    assert_eq!(fixture.finished(slow).completed_files, 1);
}

#[test]
fn transfer_change_notifications_cannot_lose_a_change_before_first_poll() {
    use futures::FutureExt as _;
    use nocterm_transfers::{CollisionPolicy, TransferService as _};
    let fixture = Fixture::new();
    let revision = fixture.runtime.revision().unwrap();
    let mut change = fixture.runtime.changed_since(revision).unwrap();
    assert!(change.as_mut().now_or_never().is_none());
    fixture
        .local
        .seed_native_file(&native("input/file"), b"data".to_vec())
        .unwrap();
    let id = fixture.upload(vec![native("input/file")], CollisionPolicy::Rename);
    futures::executor::block_on(change);
    assert!(fixture.runtime.revision().unwrap() > revision);
    fixture.finished(id);
}
