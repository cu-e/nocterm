//! Real source certificates admit before enqueue and transfer with Live publication.
use super::*;
use crate::{
    history::{
        live::{LiveEpoch, LiveOwner},
        source::{SourceLease, SourceProducer},
    },
    model::{
        ids::SessionId,
        json::deferred::DecodeLedger,
        retention::{
            RetentionAuthority,
            document::{
                DocumentRetention,
                tests::{snapshot, text},
            },
        },
    },
    session::SessionState,
};
struct Fixture {
    owner: LiveOwner,
    source: SourceContext,
    lease: SourceLease,
    _producer: SourceProducer,
    scope: DocumentRetention,
}
fn fixture() -> Fixture {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let foreground = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&foreground, "seed")).unwrap();
    let mut owner =
        LiveOwner::new(live, foreground, roles.take_prompt_dispatch().unwrap()).unwrap();
    owner.accept_epoch(LiveEpoch::new(
        SessionState::default().ticket(),
        1,
        SessionId::new("original-session"),
    ));
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let source = lease.context().unwrap();
    Fixture {
        owner,
        source,
        lease,
        _producer: producer,
        scope,
    }
}
fn notice(ledger: &DecodeLedger, length: usize) -> NativeOwned<SessionNotice> {
    let mut builder =
        crate::model::json::deferred::literal::LiteralBuilder::with_cleanup_frontier(ledger, 1)
            .unwrap();
    let mut string = builder.text().unwrap();
    let chunk = [b'x'; 4096];
    let mut remaining = length;
    while remaining != 0 {
        let count = remaining.min(chunk.len());
        string
            .append(std::str::from_utf8(&chunk[..count]).unwrap())
            .unwrap();
        remaining -= count;
    }
    builder.string(string).unwrap();
    let native = NativeOwned::text(builder.finish_native().unwrap().into_text().unwrap());
    let content = crate::model::native::content::from_text(
        crate::model::native::content::text_content(
            native,
            NativeOwned::absent(ledger).unwrap(),
            NativeOwned::absent(ledger).unwrap(),
        )
        .unwrap(),
    );
    crate::model::native::notice::notice(
        crate::model::native::controls::session_id(text(ledger, "original-session")).unwrap(),
        crate::model::native::notice::user_chunk(content),
    )
    .unwrap()
}
#[test]
fn pending_certificates_refuse_semantic_overlap_and_refund_after_original_payload_drop() {
    let fixture = fixture();
    let baseline = fixture.scope.retained_and_pending_bytes().unwrap();
    let first = fixture
        .source
        .certify_notice(notice(fixture.source.ledger(), 20 * 1024 * 1024))
        .unwrap();
    assert!(fixture.scope.retained_and_pending_bytes().unwrap() > 40 * 1024 * 1024);
    let refused = fixture
        .source
        .certify_notice(notice(fixture.source.ledger(), 13 * 1024 * 1024));
    let refused = match refused {
        Err(value) => value,
        Ok(_) => panic!("independent semantic overlap must refuse before enqueue"),
    };
    assert!(matches!(
        refused.cause(),
        SourceSemanticError::Allocation(JsonDecodeError::Limit { .. })
    ));
    assert_eq!(refused.notice().session.as_str(), "original-session");
    drop(refused);
    assert!(fixture.scope.retained_and_pending_bytes().unwrap() > 40 * 1024 * 1024);
    drop(first);
    assert_eq!(
        fixture.scope.retained_and_pending_bytes().unwrap(),
        baseline
    );
}
#[test]
fn live_publication_consumes_exact_original_certificate_even_after_source_closes() {
    let mut fixture = fixture();
    let original = fixture
        .source
        .certify_notice(notice(fixture.source.ledger(), 20 * 1024 * 1024))
        .unwrap();
    let revision = fixture.owner.revision();
    let late = notice(fixture.source.ledger(), 0);
    drop(fixture.lease);
    let admission = fixture.owner.local_admission();
    let changed = fixture
        .owner
        .apply_certified(admission, original, 9)
        .unwrap();
    assert!(changed.saved_changed());
    assert_eq!(changed.revision(), revision + 1);
    assert_eq!(fixture.owner.rows().len(), 2);
    // Leaving the consumed source projection in Q would charge another40MiB
    // and reject this otherwise valid publication under the original64MiB.
    assert!(
        fixture.scope.retained_and_pending_bytes().unwrap() < crate::history::MAX_RESIDENT_BYTES
    );
    let admission = fixture.owner.local_admission();
    fixture.owner.truncate(admission, 0).unwrap();
    assert!(fixture.scope.retained_and_pending_bytes().unwrap() < 1024 * 1024);
    let refused = fixture.source.certify_notice(late);
    assert!(refused.is_err());
}

#[test]
fn certified_agent_info_moves_original_backing_into_live_and_rejects_foreign_source() {
    use crate::model::native::NativeList;
    fn info(source: &SourceContext, name: &str) -> ManagedAgentInfo {
        let ledger = source.ledger();
        let capabilities = crate::model::native::session::capabilities(
            true, false, false, true, false, false, ledger,
        )
        .unwrap();
        let value = crate::model::native::session::agent_info(
            text(ledger, name),
            text(ledger, "version"),
            capabilities,
            NativeList::new(ledger, 0).unwrap().finish(),
        )
        .unwrap();
        source.certify_info(value).unwrap()
    }
    let mut f = fixture();
    let certificate = info(&f.source, "original agent name");
    let pointer = certificate.info().name.as_ptr();
    let saved = f.owner.revision();
    let applied = f
        .owner
        .install_info(f.owner.local_admission(), certificate)
        .unwrap();
    assert!(!applied.saved_changed());
    assert_eq!(f.owner.revision(), saved + 1);
    assert_eq!(
        f.owner.controls().unwrap().info().unwrap().name.as_ptr(),
        pointer
    );
    let other = fixture();
    let certificate = info(&other.source, "foreign");
    assert!(matches!(
        f.owner.install_info(f.owner.local_admission(), certificate),
        Err(crate::history::live::LiveError::Role)
    ));
    assert_eq!(f.owner.revision(), saved + 1);
    assert_eq!(
        f.owner.controls().unwrap().info().unwrap().name.as_ptr(),
        pointer
    );
    drop(f.lease);
    assert_eq!(
        f.owner.controls().unwrap().info().unwrap().name,
        "original agent name"
    );
}
