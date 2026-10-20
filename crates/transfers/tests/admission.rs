//! Pure transfer admission and progress rules, independent of any executor.
use nocterm_session::{Target, testing::InMemoryFs};
use nocterm_transfers::*;
use std::{error::Error as _, io, path::PathBuf, sync::Arc};

fn request() -> Request {
    Request::Upload(UploadRequest {
        sources: vec![PathBuf::from("selected.txt")],
        host: nocterm_session::HostAddress::direct(Target::new("u", "h", 22)),
        destination: "/dest".into(),
        fs: Arc::new(InMemoryFs::default()),
        collisions: CollisionPolicy::Skip,
    })
}
#[test]
fn request_bounds_are_checked_before_runtime_admission() {
    let mut request = request();
    let Request::Upload(upload) = &mut request else {
        unreachable!()
    };
    upload.sources.clear();
    assert!(matches!(
        request.validate(DiscoveryLimits::default()),
        Err(TransferError::Empty)
    ));
    let Request::Upload(upload) = &mut request else {
        unreachable!()
    };
    upload.sources = vec![PathBuf::from("file"); 4097];
    assert!(matches!(
        request.validate(DiscoveryLimits::default()),
        Err(TransferError::RequestTooLarge)
    ));
}
#[test]
fn relative_or_traversing_remote_destinations_are_rejected() {
    for destination in ["relative", "/a/../b", "/a/./b", "/a\0b"] {
        let mut request = request();
        let Request::Upload(upload) = &mut request else {
            unreachable!()
        };
        upload.destination = destination.into();
        assert!(matches!(
            request.validate(DiscoveryLimits::default()),
            Err(TransferError::InvalidDestination)
        ));
    }
}
#[test]
fn discovery_depth_visit_and_pending_name_budgets_are_independent() {
    let limits = DiscoveryLimits::with_walk_bounds(2, 3, 2, 8).unwrap();
    let mut budget = DiscoveryBudget::new(limits);
    assert!(matches!(
        budget.visit(2),
        Err(TransferCause::Limit(LimitKind::Depth))
    ));
    budget.visit(0).unwrap();
    budget.visit(1).unwrap();
    budget.visit(1).unwrap();
    assert!(matches!(
        budget.visit(0),
        Err(TransferCause::Limit(LimitKind::Visited))
    ));
    budget.hold_listing(2, 8).unwrap();
    assert!(matches!(
        budget.hold_listing(1, 0),
        Err(TransferCause::Limit(LimitKind::PendingEntries))
    ));
    budget.consumed_name(4);
    assert!(matches!(
        budget.hold_listing(0, 5),
        Err(TransferCause::Limit(LimitKind::PendingNames))
    ));
}
#[test]
fn production_limit_constructors_cannot_raise_caps() {
    assert!(QueueLimits::new(9, 32, 4, 64).is_err());
    assert!(QueueLimits::new(8, 32, 5, 64).is_err());
    assert!(StreamLimits::new(32769, 8, 100).is_err());
    assert!(StreamLimits::new(32768, 9, 100).is_err());
    assert!(DiscoveryLimits::with_walk_bounds(257, 1_000_000, 100_000, 8 * 1024 * 1024).is_err());
    assert!(QueueLimits::new(1, 1, 1, 1).is_ok());
    assert!(StreamLimits::new(1, 1, 1).is_ok());
}

#[test]
fn runtime_admission_error_clones_share_the_original_nonclone_native_leaf() {
    let error = TransferError::runtime(io::Error::new(
        io::ErrorKind::NotFound,
        "executor unavailable",
    ));
    let clone = error.clone();
    assert!(matches!(&error, TransferError::Runtime(_)));
    let original = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    let copied = clone
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert!(std::ptr::eq(original, copied));
    assert_eq!(original.kind(), io::ErrorKind::NotFound);
    assert_eq!(
        clone.to_string(),
        "Transfer runtime failed: executor unavailable"
    );
}
#[test]
fn runtime_admission_retains_original_wrapper_before_its_deeper_cause() {
    let backend = nocterm_core::PersistError::Io {
        path: "transfer-state".into(),
        source: io::Error::new(io::ErrorKind::StorageFull, "admission unavailable"),
    };
    let description = backend.to_string();
    let error = TransferError::runtime(backend);
    let clone = error.clone();
    let original = error.source().unwrap().source().unwrap();
    assert!(
        original
            .downcast_ref::<nocterm_core::PersistError>()
            .is_some()
    );
    assert!(std::ptr::eq(
        original,
        clone.source().unwrap().source().unwrap()
    ));
    assert_eq!(
        original
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::StorageFull
    );
    assert_eq!(
        error.to_string(),
        format!("Transfer runtime failed: {description}")
    );
}
