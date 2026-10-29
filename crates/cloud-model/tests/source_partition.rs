mod support;
use nocterm_cloud_model::identity::*;
use support::*;

#[test]
fn identical_display_ids_keep_different_hub_authorities_and_account_keys_partitioned() {
    let original = scope(1);
    let other_hub = AccountScope::new(
        HubIdentity::new(original.hub().instance().clone(), fingerprint(2)),
        original.account().clone(),
    );
    let other_key = AccountScope::new(
        original.hub().clone(),
        AccountIdentity::new(original.account().id().clone(), fingerprint(2)),
    );
    assert_ne!(original, other_hub);
    assert_ne!(original, other_key);
    assert_eq!(
        original,
        AccountScope::new(original.hub().clone(), original.account().clone())
    );
}

#[test]
fn identifiers_keep_literal_bytes_and_refuse_oversized_utf8_without_normalization() {
    let decomposed = NamespaceId::new("e\u{301}").unwrap();
    assert_ne!(decomposed, NamespaceId::new("é").unwrap());
    assert_eq!(decomposed.as_str(), "e\u{301}");
    assert!(NamespaceId::new("x".repeat(128)).is_ok());
    for value in [String::new(), "x".repeat(129), "界".repeat(43)] {
        assert_eq!(
            NamespaceId::new(value),
            Err(SourceIdentityError::NamespaceLength)
        );
    }
}

#[test]
fn epoch_and_revision_exhaustion_never_wraps_to_reusable_authority() {
    assert_eq!(RunEpoch::new(0), Err(IdentityError::ZeroEpoch));
    assert_eq!(
        WriterEpoch::new(u64::MAX).unwrap().next(),
        Err(IdentityError::Exhausted)
    );
    assert_eq!(
        Revision::from_u64(u64::MAX).next(),
        Err(IdentityError::Exhausted)
    );
    assert_eq!(
        Timestamp::from_millis(1).elapsed_since(Timestamp::from_millis(2)),
        None
    );
}

#[test]
fn diagnostic_identity_output_does_not_reveal_supplied_namespace_text() {
    let value = NamespaceId::new("private-account-name").unwrap();
    assert!(!format!("{value:?}").contains("private-account-name"));
}

#[test]
fn runner_key_rotation_changes_authority_without_forking_chat_storage_identity() {
    use nocterm_cloud_model::authority::RunBinding;
    let original = run(1, 1, 1, 1);
    let rotated = RunBinding::new(
        original.scope().clone(),
        original.runner(),
        RunnerKeyEpoch::new(2).unwrap(),
        original.chat(),
        original.run(),
        original.epoch(),
    );
    assert_eq!(original.identity(), rotated.identity());
    assert_eq!(original.stream().identity(), rotated.stream().identity());
    assert_ne!(original.stream(), rotated.stream());
    assert!(original.same_chat(&rotated));
    assert!(!original.same_run(&rotated));
    assert!(matches!(
        authority(original).require_same(&authority(rotated)),
        Err(nocterm_cloud_model::authority::AuthorityError::StaleRun)
    ));
}
