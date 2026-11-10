use super::*;
use crate::{
    execution_origin::ChatKey,
    local_environment::{ChatScope, LocalEnvironment, LocalEnvironmentAccess},
};
use nocterm_hubs_model::{
    HubAddress, HubId,
    source_identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, LegacyConfiguredAccount,
        LocalAccountPartition, LocalAccountSourceFactory, NamespaceId,
    },
    testing::ScriptedLocalAccountFactory,
};
use std::sync::Arc;

fn partition(authority: u8, account_key: u8) -> LocalAccountPartition {
    LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("same-instance-uuid").unwrap(),
            Fingerprint::from_bytes([authority; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new("same-account-uuid").unwrap(),
            Fingerprint::from_bytes([account_key; 32]),
        ),
    ))
}
fn scope(authority: u8, key: u8) -> ChatScope {
    ChatScope::new(
        LocalEnvironment::HubAccount(partition(authority, key)),
        ContextId::General,
    )
}
fn record(scope: ChatScope) -> SavedChat {
    let mut chat = SavedChat::for_environment("agent".into(), ID.into(), 1, scope);
    chat.entries
        .push(crate::thread::Entry::Agent("saved reply".into()));
    chat
}

#[test]
fn environment_and_project_are_independent_coordinates_of_chat_identity() {
    let a = ChatKey::in_environment(scope(1, 2), ID.into());
    let b = ChatKey::in_environment(scope(9, 2), ID.into());
    let c = ChatKey::in_environment(scope(1, 9), ID.into());
    let project: ContextId = serde_json::from_value(project()).unwrap();
    let project =
        ChatKey::in_environment(ChatScope::new(a.environment().clone(), project), ID.into());
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, project);
    assert_eq!(a.scope(), ContextId::General);
    assert_ne!(a.environment().storage_key(), b.environment().storage_key());
    assert_ne!(a.environment().storage_key(), c.environment().storage_key());
    assert_eq!(a.environment().storage_key().len(), 64);
}

#[test]
fn version_three_has_matching_bounded_environment_header_and_full_body() {
    for scope in [scope(1, 2), ChatScope::this_computer(ContextId::General)] {
        let chat = record(scope.clone());
        let bytes = encode(&chat).unwrap();
        assert!(bytes.starts_with(b"{\"version\":3,\"catalog\":"));
        let header = decode_catalog_header(&bytes, ID).unwrap();
        assert_eq!(header.chat_scope(), scope);
        assert_eq!(header.transcript_version(), TranscriptVersion::ENVIRONMENT);
        let decoded = decode(&bytes, ID).unwrap();
        assert_eq!(decoded.header.chat_scope(), scope);
        let summary = decode_summary(
            &bytes,
            ID,
            FileStamp {
                bytes: bytes.len() as u64,
                modified_nanos: 0,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(summary.chat_scope(), scope);
        assert_eq!(summary.transcript_version(), TranscriptVersion::ENVIRONMENT);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["catalog"]["environment"],
            value["chat"]["environment"]
        );
        assert_eq!(encode(&prepare(decoded).unwrap()).unwrap(), bytes);
    }
}

#[test]
fn version_three_refuses_missing_unknown_moved_or_mismatched_environment() {
    let valid = serde_json::to_value(record(scope(1, 2))).unwrap();
    let mut missing = valid.clone();
    missing["catalog"]
        .as_object_mut()
        .unwrap()
        .remove("environment");
    let mut missing_body = valid.clone();
    missing_body["chat"]
        .as_object_mut()
        .unwrap()
        .remove("environment");
    let mut changed = valid.clone();
    changed["chat"]["environment"] = serde_json::to_value(scope(9, 2).environment()).unwrap();
    let mut unknown = valid.clone();
    unknown["catalog"]["environment"] = json!({"kind":"future"});
    let mut malformed = valid.clone();
    malformed["chat"]["environment"] =
        json!({"kind":"hub_account","account":{"kind":"stable","identity":{}}});
    for value in [missing, missing_body, changed, unknown, malformed] {
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(decode(&bytes, ID).is_err());
        assert!(
            decode_summary(
                &bytes,
                ID,
                FileStamp {
                    bytes: bytes.len() as u64,
                    modified_nanos: 0
                }
            )
            .is_err()
        );
    }
}

#[test]
fn legacy_versions_never_acquire_hub_account_membership() {
    let original = decode(LEGACY, ID).unwrap();
    assert!(original.header.environment.is_this_computer());
    assert_eq!(encode(&original).unwrap(), LEGACY);
    for version in [1, 2] {
        let mut moved = record(scope(1, 2));
        moved.version = version;
        assert!(matches!(
            encode(&moved),
            Err(HistoryCodecError::LegacyEnvironment)
        ));
    }
    let mut version_two = SavedChat::for_identity("agent".into(), ID.into(), 1);
    version_two
        .entries
        .push(crate::thread::Entry::Agent("local".into()));
    let value = serde_json::to_value(&version_two).unwrap();
    assert!(value["catalog"].get("environment").is_none());
    assert_eq!(
        decode(&encode(&version_two).unwrap(), ID)
            .unwrap()
            .header
            .environment,
        LocalEnvironment::ThisComputer
    );
}

#[test]
fn new_capture_after_unlock_never_revives_an_old_a_to_b_to_a_lease() {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    assert!(factory.capture().is_err());
    factory.sign_in(partition(1, 2));
    let original = LocalEnvironmentAccess::capture(factory.capture().unwrap()).unwrap();
    let refreshed = LocalEnvironmentAccess::capture(factory.capture().unwrap()).unwrap();
    assert_eq!(original.environment(), refreshed.environment());
    assert!(original.check_current().is_ok());
    factory.sign_in(partition(1, 9));
    assert!(original.check_current().is_err());
    factory.sign_in(partition(1, 2));
    let reopened = LocalEnvironmentAccess::capture(factory.capture().unwrap()).unwrap();
    assert_eq!(original.environment(), reopened.environment());
    assert!(original.check_current().is_err());
    assert!(reopened.check_current().is_ok());
    factory.close();
    assert!(reopened.check_current().is_err());
}

#[test]
fn legacy_configured_partition_is_explicit_and_does_not_claim_stable_alias_identity() {
    let legacy = |host: &str| {
        LocalEnvironment::HubAccount(LocalAccountPartition::LegacyConfigured(
            LegacyConfiguredAccount::new(
                HubId::from(HubAddress::parse(host).unwrap()),
                NamespaceId::new("account").unwrap(),
                Fingerprint::from_bytes([8; 32]),
            ),
        ))
    };
    assert_eq!(legacy("https://HUB.home/"), legacy("hub.home:7443"));
    assert_ne!(legacy("hub.home"), legacy("alias.home"));
    assert_ne!(
        legacy("hub.home").storage_key(),
        scope(1, 2).environment().storage_key()
    );
    let mut value = serde_json::to_value(scope(1, 2).environment()).unwrap();
    value["account"]["identity"]["hub"]["instance"] = json!("x".repeat(129));
    assert!(serde_json::from_value::<LocalEnvironment>(value).is_err());
}
