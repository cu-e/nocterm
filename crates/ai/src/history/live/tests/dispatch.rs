//! A popped prompt keeps its original physical text through the original dispatch loan.
use super::*;
use crate::history::dispatch::{DispatchError, DispatchFraming};

fn framing(ledger: &DecodeLedger) -> DispatchFraming {
    DispatchFraming::new(
        crate::model::native::controls::session_id(text(ledger, "original session")).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        false,
    )
}

#[test]
fn abandoned_and_panicking_native_preparation_leave_original_queue_and_revision() {
    let mut f = fixture();
    f.owner
        .queue_text(
            f.owner.local_admission(),
            41,
            "queued original text",
            &[],
            None,
        )
        .unwrap();
    f.owner
        .set_draft(f.owner.local_admission(), Some("concurrent draft"))
        .unwrap();
    let rows = f.owner.rows().len();
    let revision = f.owner.revision();
    let producer = f.owner.dispatch().clone();
    let baseline = f._scope.retained_and_pending_bytes().unwrap();
    let pending = f
        .owner
        .stage_dispatch(f.owner.local_admission(), framing(&f.ledger), 77)
        .unwrap();
    assert_eq!(pending.payload().prompt().id(), 41);
    assert!(matches!(producer.capture(), Err(DispatchError::Busy)));
    drop(pending);
    assert_eq!(f.owner.rows().len(), rows);
    assert_eq!(f.owner.revision(), revision);
    assert_eq!(f.owner.composer().unwrap().queue()[0].id(), 41);
    assert_eq!(
        f.owner.composer().unwrap().draft().as_deref(),
        Some("concurrent draft")
    );
    assert_eq!(f._scope.retained_and_pending_bytes().unwrap(), baseline);
    drop(producer.capture().unwrap());

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _pending = f
            .owner
            .stage_dispatch(f.owner.local_admission(), framing(&f.ledger), 77)
            .unwrap();
        panic!("native staging failed");
    }));
    assert!(panic.is_err());
    assert_eq!(f.owner.rows().len(), rows);
    assert_eq!(f.owner.revision(), revision);
    assert_eq!(f.owner.composer().unwrap().queue()[0].id(), 41);
    assert_eq!(f._scope.retained_and_pending_bytes().unwrap(), baseline);
    drop(producer.capture().unwrap());
}

#[test]
fn committed_preparation_publishes_original_prompt_only_after_native_preflight() {
    let mut f = fixture();
    f.owner
        .queue_text(
            f.owner.local_admission(),
            41,
            "queued original text",
            &[],
            None,
        )
        .unwrap();
    let pointer = f.owner.composer().unwrap().queue()[0].text().as_ptr();
    let revision = f.owner.revision();
    let pending = f
        .owner
        .stage_dispatch(f.owner.local_admission(), framing(&f.ledger), 77)
        .unwrap();
    assert_eq!(pending.payload().prompt().text().as_ptr(), pointer);
    let lease = pending.commit().unwrap();
    assert_eq!(f.owner.revision(), revision + 1);
    assert!(f.owner.composer().unwrap().queue().is_empty());
    assert_eq!(lease.payload().unwrap().prompt().text().as_ptr(), pointer);
    let RowBody::User(user) = f.owner.rows().last().unwrap().view() else {
        panic!("visible User")
    };
    let crate::history::physical::ContentSlot::Text(text) = user.contents().next().unwrap() else {
        panic!("original text")
    };
    assert_eq!(text.text().as_ptr(), pointer);
}

#[test]
fn dispatched_prompt_and_visible_user_share_original_text_beyond_live_teardown() {
    let mut f = fixture();
    let admission = f.owner.local_admission();
    f.owner
        .queue_text(admission, 41, "queued original text", &[], None)
        .unwrap();
    let original = f.owner.composer().unwrap().queue()[0].text().as_ptr();
    let ledger =
        DecodeLedger::for_operation(&f._reader, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let framing = DispatchFraming::new(
        crate::model::native::controls::session_id(text(&ledger, "original session")).unwrap(),
        NativeOwned::supplied(text(&ledger, "original context")),
        NativeOwned::absent(&ledger).unwrap(),
        false,
    );
    let producer = f.owner.dispatch().clone();
    let admission = f.owner.local_admission();
    let dispatch = f.owner.prepare_dispatch(admission, framing, 77).unwrap();
    assert!(f.owner.composer().unwrap().queue().is_empty());
    let RowBody::User(user) = f.owner.rows().last().unwrap().view() else {
        panic!("visible user")
    };
    let crate::history::physical::ContentSlot::Text(text) = user.contents().next().unwrap() else {
        panic!("original text")
    };
    assert_eq!(text.text().as_ptr(), original);
    assert_eq!(
        dispatch.payload().unwrap().prompt().text().as_ptr(),
        original
    );
    assert_eq!(
        dispatch.payload().unwrap().session().as_str(),
        "original session"
    );
    assert_eq!(
        &*dispatch.payload().unwrap().context().unwrap(),
        "original context"
    );
    assert!(matches!(producer.capture(), Err(DispatchError::Busy)));
    drop(f.owner);
    assert_eq!(
        &*dispatch.payload().unwrap().prompt().text(),
        "queued original text"
    );
    assert!(matches!(producer.capture(), Err(DispatchError::Busy)));
    drop(dispatch);
    let original_capability = producer.capture().unwrap();
    assert!(original_capability.payload().is_none());
}

#[test]
fn active_scope_retains_original_queued_attachments_and_loan_until_last_alias_drop() {
    use crate::{
        history::{live::inputs::LocalItemKey, physical::LiveAttachmentSlot},
        model::native::NativeList,
    };
    let mut f = fixture();
    let mut attachments = NativeList::new(&f.ledger, 2).unwrap();
    attachments
        .push(crate::history::live::inputs::group(text(
            &f.ledger,
            "queued group",
        )))
        .unwrap();
    let key = LocalItemKey::from_bits(0x1234);
    attachments
        .push(crate::history::live::inputs::local_item(key, &f.ledger).unwrap())
        .unwrap();
    f.owner
        .replace_attachments(f.owner.local_admission(), attachments.finish())
        .unwrap();
    let LiveAttachmentSlot::Group(group) = &f.owner.inputs().unwrap().attachments()[0] else {
        panic!("original group")
    };
    let pointer = group.read().as_ptr();
    f.owner
        .queue_text(f.owner.local_admission(), 41, "queued", &[], None)
        .unwrap();
    // The current defaults change independently after this prompt captured them.
    let empty = NativeList::new(&f.ledger, 0).unwrap().finish();
    f.owner
        .replace_attachments(f.owner.local_admission(), empty)
        .unwrap();
    let producer = f.owner.dispatch().clone();
    let active = f
        .owner
        .stage_dispatch(f.owner.local_admission(), framing(&f.ledger), 77)
        .unwrap()
        .commit()
        .unwrap();
    let scope = active.scope();
    let alias = scope.clone();
    assert!(f.owner.composer().unwrap().queue().is_empty());
    assert!(f.owner.inputs().unwrap().attachments().is_empty());
    drop(active);
    drop(f.owner);
    let values = scope.live_attachments().unwrap();
    let LiveAttachmentSlot::Group(group) = &values[0] else {
        panic!("same group")
    };
    assert_eq!(group.read().as_ptr(), pointer);
    assert!(matches!(&values[1], LiveAttachmentSlot::LocalItem(value) if *value == key));
    assert!(matches!(producer.capture(), Err(DispatchError::Busy)));
    drop(scope);
    assert!(matches!(producer.capture(), Err(DispatchError::Busy)));
    drop(alias);
    assert!(producer.capture().unwrap().payload().is_none());
}
