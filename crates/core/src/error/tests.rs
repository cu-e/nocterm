use super::*;
use crate::PersistError;
use std::io;

#[derive(Debug, thiserror::Error)]
#[error("operation failed: {0}")]
struct Operation(#[source] SharedCause);

#[test]
fn typed_clone_retains_the_same_nonclone_native_leaf() {
    let cause = SharedError::from_error(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
    let cloned = cause.clone();
    assert!(std::ptr::eq(cause.as_ref(), cloned.as_ref()));
    let original = cloned
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert!(std::ptr::eq(cause.as_ref(), original));
    assert_eq!(original.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(cloned.to_string(), "denied");
    assert!(original.source().is_none());
}

#[test]
fn erased_clone_retains_native_classification_through_a_derived_source() {
    let cause = SharedCause::new(io::Error::new(io::ErrorKind::StorageFull, "full"));
    let cloned = cause.clone();
    let original = cause.downcast_ref::<io::Error>().unwrap();
    assert!(std::ptr::eq(original, cloned.downcast_ref().unwrap()));
    let error = Operation(cloned);
    let holder = error.source().unwrap();
    assert!(holder.downcast_ref::<SharedCause>().is_some());
    let leaf = holder
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert!(std::ptr::eq(original, leaf));
    assert_eq!(leaf.kind(), io::ErrorKind::StorageFull);
    assert_eq!(error.to_string(), "operation failed: full");
}

#[test]
fn typed_and_erased_nonleaf_causes_keep_the_concrete_wrapper_before_its_source() {
    let backend = || PersistError::Io {
        path: "state.toml".into(),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    };
    let typed = SharedError::from_error(backend());
    let erased = SharedCause::new(backend());
    for holder in [&typed as &dyn Error, &erased as &dyn Error] {
        let wrapper = holder.source().unwrap();
        assert!(wrapper.downcast_ref::<PersistError>().is_some());
        let native = wrapper
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap();
        assert_eq!(native.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(holder.to_string(), wrapper.to_string());
    }
}

#[test]
fn construction_from_arc_reuses_the_existing_allocation() {
    let original = Arc::new(io::Error::from(io::ErrorKind::NotFound));
    let typed = SharedError::from_arc(original.clone());
    let erased_arc: Arc<dyn Error + Send + Sync> = original.clone();
    let erased = SharedCause::from_arc(erased_arc);
    assert!(std::ptr::eq(original.as_ref(), typed.as_ref()));
    assert!(std::ptr::eq(
        original.as_ref(),
        erased.downcast_ref().unwrap()
    ));
}

#[test]
fn consuming_typed_and_erased_holders_preserves_payload_and_surviving_sources() {
    let typed = SharedError::from_error(io::Error::from(io::ErrorKind::PermissionDenied));
    let held = typed.clone();
    let payload = typed.into_arc();
    assert!(std::ptr::eq(payload.as_ref(), held.as_ref()));
    assert!(std::ptr::eq(
        payload.as_ref(),
        held.source().unwrap().downcast_ref::<io::Error>().unwrap()
    ));
    drop(payload);
    assert_eq!(held.as_ref().kind(), io::ErrorKind::PermissionDenied);

    let erased = SharedCause::new(io::Error::from(io::ErrorKind::StorageFull));
    let held = erased.clone();
    let payload = erased.into_arc();
    assert!(std::ptr::eq(
        payload.downcast_ref::<io::Error>().unwrap(),
        held.downcast_ref::<io::Error>().unwrap()
    ));
    assert!(std::ptr::eq(
        payload.downcast_ref::<io::Error>().unwrap(),
        held.source().unwrap().downcast_ref::<io::Error>().unwrap()
    ));
    drop(payload);
    assert_eq!(
        held.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::StorageFull
    );
}
