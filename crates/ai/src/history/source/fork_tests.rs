//! A named fork preserves a published row from the original native source.
use crate::{
    history::{
        live::LiveEpoch,
        physical::{RowBody, SnapshotBuilder},
    },
    model::{
        ids::SessionId,
        retention::{
            RetentionAuthority,
            document::{DocumentRetention, tests::user_text},
        },
    },
    session::SessionState,
    thread::ThreadState,
};

#[test]
fn published_native_source_rows_fork_to_an_independent_document_with_original_text() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let mut original = ThreadState::new(
        &authority,
        "agent",
        "33abddab-1ee2-4f92-8c2e-182cd401241a",
        1,
    )
    .unwrap();
    let source = original.source().capture().unwrap();
    let context = source.context().unwrap();
    let epoch = LiveEpoch::new(
        SessionState::default().ticket(),
        1,
        SessionId::new("original session"),
    );
    original.live_mut().accept_epoch(epoch.clone());
    let admission = original.live().admission(&epoch).unwrap();
    let notice = crate::model::native::notice::agent_chunk(user_text(
        context.ledger(),
        "original native source text",
    ));
    let _ = original
        .live_mut()
        .apply_owned(admission, notice, 2)
        .unwrap();
    let RowBody::Agent(text) = original.rows()[0].view() else {
        panic!("native agent row")
    };
    let original_pointer = text.read().as_ptr();
    let (target, mut target_roles) = DocumentRetention::new(&authority).unwrap();
    let mut foreground = target_roles.take_foreground().unwrap();
    let mut target_live = target_roles.take_live().unwrap();
    let target_baseline = target.retained_and_pending_bytes().unwrap();
    let builder = SnapshotBuilder::from_fork_role(&mut foreground, original.live().role())
        .expect("published original source can fork through the authentic named intake");
    target_live.replace(builder.finish(None).unwrap()).unwrap();
    foreground.clear_source();
    let RowBody::Agent(text) = target_live.snapshot().unwrap().rows()[0].view() else {
        panic!("forked native agent row")
    };
    assert_eq!(text.read().as_ptr(), original_pointer);
    assert_eq!(&*text.read(), "original native source text");
    let before_origin_teardown = target.retained_and_pending_bytes().unwrap();
    drop(source);
    drop(context);
    drop(original);
    assert_eq!(
        target.retained_and_pending_bytes().unwrap(),
        before_origin_teardown,
        "fork still owns actual original weak ledger, phase and account headers"
    );
    let RowBody::Agent(text) = target_live.snapshot().unwrap().rows()[0].view() else {
        panic!("surviving fork")
    };
    assert_eq!(text.read().as_ptr(), original_pointer);
    assert_eq!(&*text.read(), "original native source text");
    target_live.clear();
    foreground.clear_source();
    assert_eq!(
        target.retained_and_pending_bytes().unwrap(),
        target_baseline,
        "actual fork payload and weak headers retire to the original target baseline"
    );
}

#[test]
fn a_thread_fork_keeps_a_row_prefix_under_its_own_identity() {
    use crate::thread::ForkStaging;
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let mut original = ThreadState::new(
        &authority,
        "agent",
        "33abddab-1ee2-4f92-8c2e-182cd401241a",
        1,
    )
    .unwrap();
    let source = original.source().capture().unwrap();
    let context = source.context().unwrap();
    let epoch = LiveEpoch::new(
        SessionState::default().ticket(),
        1,
        SessionId::new("original session"),
    );
    original.live_mut().accept_epoch(epoch.clone());
    for (index, text) in ["first row", "second row"].into_iter().enumerate() {
        let admission = original.live().admission(&epoch).unwrap();
        let chunk = user_text(context.ledger(), text);
        let notice = if index == 0 {
            crate::model::native::notice::user_chunk(chunk)
        } else {
            crate::model::native::notice::agent_chunk(chunk)
        };
        let _ = original
            .live_mut()
            .apply_owned(admission, notice, 2 + index as u64)
            .unwrap();
    }
    let fork = ForkStaging::new(&original, 1)
        .finish(
            &authority,
            "6d1c9f6a-7a2c-4a3e-8a4f-0f3a1d4b2c10",
            "Original (fork)",
            true,
            9,
        )
        .unwrap();
    let snapshot = fork.live().role().snapshot().unwrap();
    assert_eq!(&*snapshot.id(), "6d1c9f6a-7a2c-4a3e-8a4f-0f3a1d4b2c10");
    assert_eq!(
        snapshot.metadata().name().as_deref(),
        Some("Original (fork)")
    );
    assert!(snapshot.resume().pending_history());
    assert!(snapshot.resume().session_id().is_none());
    assert_eq!(fork.rows().len(), 1);
    assert_eq!(original.rows().len(), 2);
    assert!(fork.live().composer().unwrap().queue().is_empty());
    let RowBody::User(forked) = fork.rows()[0].view() else {
        panic!("forked row")
    };
    let RowBody::User(kept) = original.rows()[0].view() else {
        panic!("original row")
    };
    let (
        Some(crate::history::physical::ContentSlot::Text(forked)),
        Some(crate::history::physical::ContentSlot::Text(kept)),
    ) = (forked.contents().next(), kept.contents().next())
    else {
        panic!("text rows")
    };
    assert_eq!(forked.text().as_ptr(), kept.text().as_ptr());
}
