//! Paid pure construction preserves original policy and retires removed backings.
use super::*;
use crate::{
    history::source::SourceProducer,
    model::retention::{RetentionAuthority, document::DocumentRetention},
};
fn producer() -> SourceProducer {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    SourceProducer::new(roles.take_source().unwrap()).unwrap()
}
#[test]
fn managed_policy_preserves_all_original_paths_order_and_distinct_credential_copies() {
    let producer = producer();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let private = [PathBuf::from("private/../root")];
    let shared = [PathBuf::from("shared//socket")];
    for home in [None, Some(Path::new("/home/é"))] {
        let baseline = context.ledger().live_requested_bytes();
        let actual = crate::model::native::workspace::sandbox_policy(
            context.ledger(),
            Path::new("/work/../workspace"),
            home,
            &private,
            &shared,
        )
        .unwrap();
        assert_eq!(
            actual.get(),
            &SandboxPolicy::new(Path::new("/work/../workspace"), home, &private, &shared)
        );
        if home.is_some() {
            let credential = &actual.get().credentials[0];
            let hidden = &actual.get().hidden[private.len()];
            assert_eq!(credential, hidden);
            assert_ne!(
                credential.as_os_str().as_encoded_bytes().as_ptr(),
                hidden.as_os_str().as_encoded_bytes().as_ptr()
            );
        }
        drop(actual);
        assert_eq!(context.ledger().live_requested_bytes(), baseline);
    }
}
#[test]
fn extracting_canonical_root_retires_original_path_credit_after_its_physical_drop() {
    let producer = producer();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let baseline = context.ledger().live_requested_bytes();
    let original = PathBuf::from("x".repeat(4096));
    let resolved = crate::model::native::workspace::resolved_path(
        context.copy_path(&original).unwrap(),
        context.copy_path(Path::new("/r")).unwrap(),
        PathKind::Directory,
    )
    .unwrap();
    assert_eq!(context.ledger().live_requested_bytes(), baseline + 4096 + 2);
    let root = crate::model::native::workspace::into_resolved_path(resolved).unwrap();
    assert_eq!(root.as_path(), Path::new("/r"));
    assert_eq!(context.ledger().live_requested_bytes(), baseline + 2);
    drop(root);
    assert_eq!(context.ledger().live_requested_bytes(), baseline);
}
