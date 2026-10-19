//! Source capabilities remain genuine when a parent driver retires before its recipe child.
use super::*;
use crate::loading::{
    json_fields::Key,
    native::schema::{Schema, Scope},
};
use nocterm_ai::{history::MAX_RESIDENT_BYTES, model::retention::RetentionAuthority};
#[test]
fn descendant_recipe_retains_parent_and_reused_slots_reject_old_generations() {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("value.json");
    // The held string decodes to another quoted string; that child decodes to null.
    std::fs::write(&path, r#"{"rawInput":"\"null\""}"#).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_source(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            Scope::scan(
                reader,
                0,
                Schema::new(&[Key::RawInput], &[Key::RawInput], 1),
            )
        })
        .unwrap()
        .transpose()
        .unwrap();
    let root = SemanticSpan::held(
        plan.payload().required(Key::RawInput).unwrap().span(),
        false,
    );
    let mut sources = Sources::new(&plan, &mut held, &ledger).unwrap();
    let parent = sources.decoded(&root).unwrap();
    let child = sources.decoded(&parent).unwrap();
    sources.retire(parent.clone()).unwrap();
    let mut bytes = [0; 4];
    sources
        .replay(&child, |reader| {
            reader
                .read_exact(&mut bytes)
                .map_err(|_| ValueError::Punctuation)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(&bytes, b"null");
    sources.retire(child.clone()).unwrap();
    let replacement = sources.decoded(&root).unwrap();
    assert!(matches!(
        sources.replay(&parent, |_| Ok(())),
        Err(ValueError::ForeignSource)
    ));
    assert!(matches!(
        sources.replay(&child, |_| Ok(())),
        Err(ValueError::ForeignSource)
    ));
    sources.retire(replacement).unwrap();
}

#[test]
fn recipe_replay_authenticates_changed_physical_source_before_acceptance() {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("value.json");
    std::fs::write(&path, r#"{"rawInput":"null"}"#).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_source(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            Scope::scan(
                reader,
                0,
                Schema::new(&[Key::RawInput], &[Key::RawInput], 1),
            )
        })
        .unwrap()
        .transpose()
        .unwrap();
    let root = SemanticSpan::held(
        plan.payload().required(Key::RawInput).unwrap().span(),
        false,
    );
    let mut sources = Sources::new(&plan, &mut held, &ledger).unwrap();
    let decoded = sources.decoded(&root).unwrap();
    std::fs::write(&path, r#"{"rawInput":"true"}"#).unwrap();
    let result = sources.replay(&decoded, |reader| {
        let mut provisional = [0; 4];
        reader
            .read_exact(&mut provisional)
            .map_err(|_| ValueError::Punctuation)?;
        Ok(provisional)
    });
    assert!(matches!(
        result,
        Err(ValueError::Held(
            crate::loading::held_file::HeldReadError::ChangedDocument
        ))
    ));
}
