//! Original source loan reuse and pending payload lifetime through teardown.
use super::*;
use crate::model::{
    native::NativeBytes,
    retention::{RetentionAuthority, document::DocumentRetention},
};

#[test]
fn original_source_is_exclusive_and_old_context_cannot_allocate_after_teardown() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    assert!(matches!(producer.capture(), Err(SourceError::Unavailable)));
    let context = lease.context().unwrap();
    let mut bytes = NativeBytes::new(context.ledger(), 4096).unwrap();
    bytes.append(b"original source").unwrap();
    let payload = bytes.finish();
    drop(lease);
    assert_eq!(
        payload.ledger().live_requested_bytes(),
        context.ledger().live_requested_bytes()
    );
    assert!(context.reserve_native(Layout::new::<u64>()).is_err());
    let next = producer.capture().unwrap();
    let current = next.context().unwrap();
    assert!(context.same_document(&current));
    assert!(!context.same_source(&current));
    drop(current.reserve_native(Layout::new::<u64>()).unwrap());
    let before_drop = scope.retained_and_pending_bytes().unwrap();
    drop(payload);
    assert_eq!(
        scope.retained_and_pending_bytes().unwrap() + 4096,
        before_drop
    );
    drop(next);
    assert!(current.reserve_native(Layout::new::<u64>()).is_err());
    assert!(producer.capture().is_ok());
}

#[test]
fn original_layout_refusal_preserves_other_prepaid_source_buffers() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let mut original = NativeBytes::new(context.ledger(), 16).unwrap();
    original.append(b"kept").unwrap();
    let before = scope.retained_and_pending_bytes().unwrap();
    assert!(
        context
            .reserve_native(Layout::array::<u8>(crate::history::MAX_RESIDENT_BYTES).unwrap())
            .is_err()
    );
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    assert_eq!(original.as_slice(), b"kept");
    drop(original);
    assert_eq!(scope.retained_and_pending_bytes().unwrap() + 16, before);
}

/// Builds the actual outgoing field under the original source's ledger,
/// without an unaccounted large fixture String or a cloned DTO graph.
fn large_output_record(
    ledger: &crate::model::json::deferred::DecodeLedger,
    bytes: usize,
) -> crate::model::native::NativeOwned<crate::model::tools::ToolRecord> {
    use crate::model::{
        json::deferred::literal::LiteralBuilder,
        native::{NativeList, NativeOwned},
        retention::document::tests::text,
        tools::{ToolKind, ToolStatus},
    };
    let mut literal = LiteralBuilder::with_cleanup_frontier(ledger, 1).unwrap();
    let mut output = literal.exact_text(bytes).unwrap();
    let chunk = [b'x'; 4096];
    let chunk = std::str::from_utf8(&chunk).unwrap();
    for _ in 0..bytes / chunk.len() {
        output.append(chunk).unwrap();
    }
    literal.string(output.finish().unwrap()).unwrap();
    let output = NativeOwned::text(literal.finish_native().unwrap().into_text().unwrap());
    let content = crate::model::native::content::from_text(
        crate::model::native::content::text_content(
            output,
            NativeOwned::absent(ledger).unwrap(),
            NativeOwned::absent(ledger).unwrap(),
        )
        .unwrap(),
    );
    let mut outputs = NativeList::new(ledger, 1).unwrap();
    outputs
        .push(
            crate::model::native::tools::content_output(
                content,
                NativeOwned::absent(ledger).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let header = crate::model::native::tools::header(
        crate::model::native::tools::tool_id(text(ledger, "call-1")).unwrap(),
        text(ledger, "Original title"),
        NativeOwned::absent(ledger).unwrap(),
        ToolKind::Read,
        ToolStatus::Running,
    )
    .unwrap();
    crate::model::native::tools::record(
        header,
        outputs.finish(),
        NativeList::new(ledger, 0).unwrap().finish(),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
    )
    .unwrap()
}
fn cleared_output_patch(
    ledger: &crate::model::json::deferred::DecodeLedger,
) -> crate::model::native::NativeOwned<crate::model::tools::ToolPatch> {
    use crate::model::{native::NativeList, retention::document::tests::text};
    crate::model::native::patch::with_output(
        crate::model::native::patch::tool_patch(
            crate::model::native::tools::tool_id(text(ledger, "call-1")).unwrap(),
            None,
            None,
        ),
        NativeList::new(ledger, 0).unwrap().finish(),
    )
    .unwrap()
}

#[test]
fn replaced_tool_output_returns_source_capacity_while_original_title_survives() {
    use crate::{
        history::{
            live::{LiveEpoch, LiveOwner, LiveRouted},
            physical::RowBody,
        },
        model::{ids::SessionId, retention::document::tests::snapshot},
        session::SessionState,
    };
    const OLD_OUTPUT_BYTES: usize = 12 * 1024 * 1024;
    const NEXT_SOURCE_BYTES: usize = 8 * 1024 * 1024;
    let authority = RetentionAuthority::new(crate::history::MAX_RESIDENT_BYTES).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let foreground = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&foreground, "seed")).unwrap();
    let mut owner =
        LiveOwner::new(live, foreground, roles.take_prompt_dispatch().unwrap()).unwrap();
    let epoch = LiveEpoch::new(
        SessionState::default().ticket(),
        1,
        SessionId::new("source"),
    );
    owner.accept_epoch(epoch.clone());
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let ledger = context.ledger();

    let record = large_output_record(ledger, OLD_OUTPUT_BYTES);
    let admission = owner.admission(&epoch).unwrap();
    assert!(matches!(
        owner
            .apply_owned(
                admission,
                crate::model::native::notice::tool_started(record),
                1
            )
            .unwrap(),
        LiveRouted::Applied(_)
    ));
    let before = scope.retained_and_pending_bytes().unwrap();
    let original_title = match owner.rows()[1].view() {
        RowBody::Tool(tool) => tool.title().as_ptr(),
        _ => panic!("tool row"),
    };
    let patch = crate::model::native::notice::tool_changed(cleared_output_patch(ledger));
    let _ = owner
        .apply_owned(owner.admission(&epoch).unwrap(), patch, 2)
        .unwrap();
    let RowBody::Tool(tool) = owner.rows()[1].view() else {
        panic!("tool row")
    };
    assert!(tool.output().is_empty());
    assert_eq!(&*tool.title(), "Original title");
    assert_eq!(tool.title().as_ptr(), original_title);
    let after = scope.retained_and_pending_bytes().unwrap();
    assert!(
        before >= after + OLD_OUTPUT_BYTES,
        "old output remains retained"
    );
    assert!(
        after < 1024 * 1024,
        "unexpected surviving physical document: {after}"
    );

    // The unchanged title aliases its real original field bundle. It must not
    // pin exclusive capacity belonging to the physically discarded output.
    // Each8MiB source buffer fits the existing16MiB ACP line rule, and all
    // seven fit together under the same document's unchanged64MiB policy.
    let mut next = Vec::with_capacity(7);
    for position in 0..7 {
        let buffer = NativeBytes::new(ledger, NEXT_SOURCE_BYTES);
        assert!(
            buffer.is_ok(),
            "a retired output still consumes source capacity at buffer {position}: {:?}; live requested={}; physical census={after}",
            buffer.as_ref().err(),
            ledger.live_requested_bytes(),
        );
        next.push(buffer.unwrap());
    }
    drop(next);
    assert!(owner.admission(&epoch).is_ok());
}
