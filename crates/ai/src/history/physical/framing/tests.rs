//! Named resume edits keep the actual unchanged physical text owners.
use super::*;
use crate::model::{
    json::deferred::DecodeLedger,
    native::NativeChoices,
    retention::{
        RetentionAuthority,
        document::{
            DocumentRetention,
            tests::{snapshot, text},
        },
    },
};
/// Session, workdir, sorted choices `a`/`z` and mode, all original text owners.
fn original_resume(ledger: &DecodeLedger) -> NativeOwned<crate::history::ResumeDescriptor> {
    let mut choices = NativeChoices::new(ledger, 2).unwrap();
    choices
        .push(
            text(ledger, "a"),
            crate::model::native::choices::text_value(text(ledger, "original-a")),
        )
        .unwrap();
    choices
        .push(
            text(ledger, "z"),
            crate::model::native::choices::text_value(text(ledger, "original-z")),
        )
        .unwrap();
    crate::history::native::document::resume(
        NativeOwned::supplied(text(ledger, "original-session")),
        NativeOwned::supplied(
            crate::model::native::tools::path(text(ledger, "/tmp/original")).unwrap(),
        ),
        choices.finish().unwrap(),
        NativeOwned::supplied(text(ledger, "original-mode")),
        false,
    )
    .unwrap()
}
#[test]
fn local_resume_changes_alias_other_fields_and_keep_original_sorted_choice_order() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let role = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&role, "seed")).unwrap();
    let ledger = DecodeLedger::for_operation(&role, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let native = original_resume(&ledger);
    let mut source = NativeSource::new(native, &role).unwrap();
    let original = source.original().clone();
    let mut builder = SlabBuilder::new(&role).unwrap();
    let body = ResumeSlot::new(source.take(), &mut builder, source.pool()).unwrap();
    let original = BundleSlot::new(BundleOwner::build(body, builder, &original).unwrap());
    let changed = original
        .body()
        .replacing_choice(
            text(&ledger, "a"),
            crate::model::native::choices::text_value(text(&ledger, "new-a")),
            &role,
        )
        .unwrap();
    assert_eq!(
        changed.body().session_id().unwrap().as_ptr(),
        original.body().session_id().unwrap().as_ptr()
    );
    assert_eq!(
        changed.body().mode().unwrap().as_ptr(),
        original.body().mode().unwrap().as_ptr()
    );
    assert_eq!(
        changed.body().workdir().unwrap().as_os_str(),
        original.body().workdir().unwrap().as_os_str()
    );
    assert_eq!(
        changed.body().config()[0].key().as_ptr(),
        original.body().config()[0].key().as_ptr()
    );
    let ChoiceValueSlot::Value(old_z) = original.body().config()[1].value() else {
        panic!("choice")
    };
    let ChoiceValueSlot::Value(new_z) = changed.body().config()[1].value() else {
        panic!("choice")
    };
    assert_eq!(old_z.read().as_ptr(), new_z.read().as_ptr());
    let inserted = changed
        .body()
        .replacing_choice(
            text(&ledger, "m"),
            crate::model::native::choices::flag(true, &ledger).unwrap(),
            &role,
        )
        .unwrap();
    assert_eq!(
        inserted
            .body()
            .config()
            .iter()
            .map(|choice| choice.key().to_string())
            .collect::<Vec<_>>(),
        ["a", "m", "z"]
    );
    let cleared = inserted
        .body()
        .clear_session(true, &role, inserted.original_credit())
        .unwrap();
    assert!(cleared.body().session_id().is_none());
    assert!(cleared.body().pending_history());
    assert_eq!(&*cleared.body().mode().unwrap(), "original-mode");
    assert_eq!(&*original.body().session_id().unwrap(), "original-session");
}
