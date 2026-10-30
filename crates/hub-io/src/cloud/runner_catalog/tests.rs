//! Actual public purpose crypto and native pagination/ownership actions, no live-client fake.
use super::*;
use super::{
    page::PageSequence,
    source::{Observed, PageStamp},
};
use crate::cloud::head_mapping::PinnedHeadMapper;
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    identity::*,
    journal::{
        JournalCatalogBinding, PublicationAllocation, PublicationBudget, PublicationWorkBudget,
    },
};
use nocterm_hub_model::cloud::{
    CloudNamespace, CloudTextId, CommandIssuer, RunnerCoordinate,
    capabilities::CapabilityWindow,
    head::{
        CatalogHeadPage, ChatHead, HeadProbeKeys, HeadProbeScope, HeadQuery, HeadTarget, HeadValue,
        HeadWitness,
    },
};
use nocterm_keys::{
    AccountKeyPair, DeviceKey,
    cloud::{RunnerEncryptionKey, RunnerSigningKey, open_head_witness, seal_head_witness},
};
use sha2::{Digest, Sha256};

struct PurposeKeys {
    account: AccountKeyPair,
    encryption: RunnerEncryptionKey,
    signing: RunnerSigningKey,
    device: DeviceKey,
}
impl PurposeKeys {
    fn new() -> Self {
        Self {
            account: AccountKeyPair::generate().unwrap(),
            encryption: RunnerEncryptionKey::generate().unwrap(),
            signing: RunnerSigningKey::generate().unwrap(),
            device: DeviceKey::generate().unwrap(),
        }
    }
    fn query(&self, target: HeadTarget) -> HeadQuery {
        HeadQuery {
            scope: HeadProbeScope {
                namespace: CloudNamespace {
                    hub_instance_id: CloudTextId::new("catalog-test-hub").unwrap(),
                    account_id: CloudTextId::new("catalog-account").unwrap(),
                },
                runner: RunnerCoordinate {
                    id: [1; 16],
                    key_epoch: 2,
                },
                keys: HeadProbeKeys {
                    account: self.account.public_key().to_bytes(),
                    runner_encryption: self.encryption.public_key().to_bytes(),
                    runner_signing: self.signing.public_key(),
                },
            },
            issuer: CommandIssuer {
                device_id: CloudTextId::new("catalog-device").unwrap(),
                signing_key: self.device.public_key(),
                access_token_digest: [3; 32],
            },
            probe_id: nocterm_keys::random_bytes().unwrap(),
            challenge: nocterm_keys::random_bytes().unwrap(),
            window: CapabilityWindow {
                issued_at_ms: 100_000,
                expires_at_ms: 110_000,
            },
            target,
        }
    }
    fn opened(
        &self,
        query: HeadQuery,
        page: CatalogHeadPage,
    ) -> nocterm_hub_model::cloud::head::SignedHeadWitness {
        let signed = self
            .signing
            .attest_head(HeadWitness {
                query: query.clone(),
                observed_at_ms: 100_001,
                value: HeadValue::Catalog(Box::new(page)),
            })
            .unwrap();
        let ciphertext = seal_head_witness(&self.encryption, &signed).unwrap();
        let result = open_head_witness(&self.account, &query, &ciphertext, 100_002).unwrap();
        let mut changed = ciphertext;
        *changed.last_mut().unwrap() ^= 1;
        assert!(open_head_witness(&self.account, &query, &changed, 100_002).is_err());
        result
    }
    fn binding(&self) -> JournalCatalogBinding {
        JournalCatalogBinding::new(
            AccountScope::new(
                HubIdentity::new(
                    NamespaceId::new("catalog-test-hub").unwrap(),
                    Fingerprint::from_bytes([3; 32]),
                ),
                AccountIdentity::new(
                    NamespaceId::new("catalog-account").unwrap(),
                    Fingerprint::from_bytes(
                        Sha256::digest(self.account.public_key().to_bytes()).into(),
                    ),
                ),
            ),
            RunnerId::from_bytes([1; 16]),
            RunnerKeyEpoch::new(2).unwrap(),
        )
    }
}
fn row(id: u8, deleted: bool) -> ChatHead {
    if deleted {
        return ChatHead {
            chat_id: [id; 16],
            chat_key_epoch: 13,
            current_writer: Some(1),
            control_revision: 1,
            deleted: true,
            head: Some(nocterm_hub_model::cloud::head::HeadRecordAnchor {
                kind: nocterm_hub_model::cloud::RecordKind::Event,
                position: nocterm_hub_model::cloud::RecordPosition {
                    writer_epoch: 1,
                    sequence: 1,
                    journal_sequence: 1,
                    revision: 1,
                },
                id: [7; 16],
                run: None,
                ciphertext_digest: [8; 32],
            }),
            retained_from: 2,
            snapshot: None,
        };
    }
    ChatHead {
        chat_id: [id; 16],
        chat_key_epoch: 13,
        current_writer: None,
        control_revision: 0,
        deleted,
        head: None,
        retained_from: 0,
        snapshot: None,
    }
}
#[test]
fn runner_catalog_actual_account_crypto_preserves_no_key_heads_and_strict_page_continuity() {
    let keys = PurposeKeys::new();
    let first = PageSequence::new(None);
    let opened = keys.opened(
        keys.query(first.target()),
        CatalogHeadPage {
            revision: 9,
            digest: [5; 32],
            total_chats: 2,
            after: None,
            chats: vec![row(1, false)],
            has_more: true,
        },
    );
    let HeadValue::Catalog(page) = &opened.witness().value else {
        panic!("catalog")
    };
    assert_eq!(first.validate(page).unwrap().revision, 9);
    let mapper = PinnedHeadMapper::new(keys.binding());
    let native = mapper.head(&page.chats[0]).unwrap();
    assert_eq!(native.key_epoch().get(), 13);
    assert!(native.facts().writer.is_none() && native.facts().head.is_none());
    let next = PageSequence::new(PageSequence::continuation(page));
    let HeadTarget::Catalog { after } = next.target() else {
        panic!("catalog")
    };
    let final_page = CatalogHeadPage {
        revision: 9,
        digest: [5; 32],
        total_chats: 2,
        after,
        chats: vec![row(2, true)],
        has_more: false,
    };
    let verified = keys.opened(keys.query(next.target()), final_page.clone());
    let HeadValue::Catalog(last) = &verified.witness().value else {
        panic!("catalog")
    };
    next.validate(last).unwrap();
    assert!(mapper.head(&last.chats[0]).unwrap().facts().deleted);
    assert!(PageSequence::continuation(last).is_none());
    // Still valid in the public codec, but conflicts with the original page's signed total.
    let changed = CatalogHeadPage {
        total_chats: 3,
        has_more: true,
        ..final_page
    };
    let verified = keys.opened(keys.query(next.target()), changed);
    let HeadValue::Catalog(changed) = &verified.witness().value else {
        panic!("catalog")
    };
    assert!(matches!(
        next.validate(changed),
        Err(RunnerCatalogError::Revision)
    ));
}
#[test]
fn runner_catalog_revision_and_clock_cannot_publish_a_lower_or_conflicting_catalog() {
    let mut observed = Observed::default();
    observed.observe(Timestamp::from_millis(100)).unwrap();
    let original = PageStamp {
        revision: 10,
        root: Fingerprint::from_bytes([2; 32]),
        total: 8,
    };
    observed.accept(original).unwrap();
    for changed in [
        PageStamp {
            revision: 9,
            ..original
        },
        PageStamp {
            root: Fingerprint::from_bytes([3; 32]),
            ..original
        },
        PageStamp {
            total: 9,
            ..original
        },
    ] {
        assert!(matches!(
            observed.accept(changed),
            Err(RunnerCatalogError::Revision)
        ));
    }
    observed.accept(original).unwrap();
    assert!(matches!(
        observed.observe(Timestamp::from_millis(99)),
        Err(RunnerCatalogError::Clock)
    ));
    assert!(matches!(
        observed.observe(Timestamp::from_millis(101)),
        Err(RunnerCatalogError::Clock)
    ));
    assert!(matches!(
        observed.accept(PageStamp {
            revision: 11,
            ..original
        }),
        Err(RunnerCatalogError::Clock)
    ));
}
#[test]
fn runner_catalog_foreign_error_keeps_its_actual_work_until_final_public_cause_drop() {
    assert!(matches!(
        sdk(CloudError::Unsupported),
        RunnerCatalogError::Unsupported(_)
    ));
    let bytes = runner_catalog_work_bytes();
    let budget = PublicationBudget::maintenance(IntakePoolLimits::new(bytes, 1, 1).unwrap());
    let allocation = budget
        .try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
        .unwrap();
    let error = retained(sdk(CloudError::Protocol), allocation);
    let mut cursor: &(dyn std::error::Error + 'static) = &error;
    let mut shared = None;
    while let Some(source) = cursor.source() {
        if let Some(cause) = source.downcast_ref::<SharedCause>() {
            shared = Some(cause.clone());
            break;
        }
        cursor = source;
    }
    let shared = shared.expect("typed SDK/backend cause remains inspectable");
    drop(error);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
            .is_err()
    );
    drop(shared);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
            .is_ok()
    );
}
