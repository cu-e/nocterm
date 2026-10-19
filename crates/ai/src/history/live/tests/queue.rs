//! Queued submission preserves original ownership and refusal behavior.
use super::*;
use crate::{
    history::physical::{ComposerInputEdit, LiveAttachmentSlot},
    model::native::NativeList,
};

fn attach_group(f: &mut Fixture, name: &str) {
    let mut values = NativeList::new(&f.ledger, 1).unwrap();
    values
        .push(crate::history::live::inputs::group(text(&f.ledger, name)))
        .unwrap();
    f.owner
        .replace_attachments(f.owner.local_admission(), values.finish())
        .unwrap();
}

fn group(values: &[LiveAttachmentSlot]) -> String {
    let LiveAttachmentSlot::Group(text) = &values[0] else {
        panic!("original group")
    };
    text.read().to_string()
}

#[test]
fn editing_original_prompt_and_restart_restore_original_default_scope() {
    let mut f = fixture();
    attach_group(&mut f, "queued group");
    f.owner
        .queue_text(
            f.owner.local_admission(),
            41,
            "queued original text",
            &[],
            None,
        )
        .unwrap();
    attach_group(&mut f, "default group");
    let revision = f.owner.revision();
    assert!(
        !f.owner
            .edit_inputs(
                f.owner.local_admission(),
                ComposerInputEdit::BeginOriginal(41)
            )
            .unwrap()
            .saved_changed()
    );
    let inputs = f.owner.inputs().unwrap();
    assert!(inputs.editing());
    assert_eq!(group(inputs.attachments()), "queued group");
    assert_eq!(group(inputs.default_attachments()), "default group");
    f.owner
        .edit_inputs(f.owner.local_admission(), ComposerInputEdit::End)
        .unwrap();
    assert!(!f.owner.inputs().unwrap().editing());
    assert_eq!(
        group(f.owner.inputs().unwrap().attachments()),
        "default group"
    );
    f.owner
        .edit_inputs(
            f.owner.local_admission(),
            ComposerInputEdit::BeginOriginal(41),
        )
        .unwrap();
    f.owner
        .edit_inputs(f.owner.local_admission(), ComposerInputEdit::Restart)
        .unwrap();
    assert!(!f.owner.inputs().unwrap().editing());
    assert_eq!(
        group(f.owner.inputs().unwrap().attachments()),
        "default group"
    );
    assert_eq!(f.owner.composer().unwrap().queue()[0].id(), 41);
    assert_eq!(f.owner.revision(), revision + 4);
    let revision = f.owner.revision();
    assert!(matches!(
        f.owner.edit_inputs(
            f.owner.local_admission(),
            ComposerInputEdit::BeginOriginal(99)
        ),
        Err(LiveError::Physical(PhysicalError::QueuedPromptMissing))
    ));
    assert_eq!(f.owner.revision(), revision);
    f.owner
        .edit_inputs(f.owner.local_admission(), ComposerInputEdit::Reset)
        .unwrap();
    assert!(f.owner.inputs().unwrap().attachments().is_empty());
    assert_eq!(f.owner.composer().unwrap().queue()[0].id(), 41);
}

/// Addresses of the retained resume session and mode text.
fn resume_text(f: &Fixture) -> (*const u8, *const u8) {
    let snapshot = f.owner.role().snapshot().unwrap();
    let resume = snapshot.resume();
    (
        resume.session_id().unwrap().as_ptr(),
        resume.mode().unwrap().as_ptr(),
    )
}
fn choose_model(f: &mut Fixture) -> bool {
    f.owner
        .replace_resume_choice(
            f.owner.local_admission(),
            text(&f.ledger, "model"),
            crate::model::native::choices::text_value(text(&f.ledger, "chosen model")),
        )
        .unwrap()
        .saved_changed()
}

#[test]
fn resume_overlay_changes_share_unchanged_text_and_noops_preserve_revision() {
    let mut f = fixture();
    f.owner
        .set_resume_session(f.owner.local_admission(), Some("original session"))
        .unwrap();
    f.owner
        .set_resume_mode(f.owner.local_admission(), Some("original mode"))
        .unwrap();
    let original = resume_text(&f);
    choose_model(&mut f);
    assert_eq!(resume_text(&f), original);
    let revision = f.owner.revision();
    assert!(
        !f.owner
            .set_resume_session(f.owner.local_admission(), Some("original session"))
            .unwrap()
            .saved_changed()
    );
    assert!(!choose_model(&mut f));
    assert_eq!(f.owner.revision(), revision);
}

#[test]
fn repeated_pending_history_marker_preserves_revision() {
    let mut f = fixture();
    f.owner
        .set_pending_history(f.owner.local_admission(), true)
        .unwrap();
    assert!(
        f.owner
            .role()
            .snapshot()
            .unwrap()
            .resume()
            .pending_history()
    );
    let revision = f.owner.revision();
    assert!(
        !f.owner
            .set_pending_history(f.owner.local_admission(), true)
            .unwrap()
            .saved_changed()
    );
    assert_eq!(f.owner.revision(), revision);
}

#[test]
fn submitted_queue_owns_original_text_and_replacement_refusal_preserves_draft() {
    let mut f = fixture();
    let admission = f.owner.local_admission();
    f.owner.set_draft(admission, Some("unsent draft")).unwrap();
    let admission = f.owner.local_admission();
    let (accepted, replaced) = f
        .owner
        .queue_text(
            admission,
            7,
            "original prompt",
            &[inputs::SavedAttachmentRef::Connection(
                "original connection",
            )],
            None,
        )
        .unwrap();
    assert!(accepted.saved_changed());
    assert!(!replaced);
    let composer = f.owner.composer().unwrap();
    assert!(composer.draft().is_none());
    assert_eq!(composer.queue().len(), 1);
    assert_eq!(composer.queue()[0].id(), 7);
    assert_eq!(&*composer.queue()[0].text(), "original prompt");
    let admission = f.owner.local_admission();
    f.owner
        .prepare_saved_candidate(admission, &mut f.candidate)
        .unwrap();
    let original = f.candidate.snapshot().unwrap().composer().queue()[0].text();
    let admission = f.owner.local_admission();
    f.owner
        .set_draft(admission, Some("replacement draft"))
        .unwrap();
    let revision = f.owner.revision();
    let admission = f.owner.local_admission();
    assert!(matches!(
        f.owner.queue_text(admission, 8, "missing", &[], Some(99)),
        Err(LiveError::Physical(PhysicalError::QueuedPromptMissing))
    ));
    assert_eq!(f.owner.revision(), revision);
    assert_eq!(
        &*f.owner.composer().unwrap().draft().unwrap(),
        "replacement draft"
    );
    let admission = f.owner.local_admission();
    let (_, replaced) = f
        .owner
        .queue_text(admission, 8, "changed prompt", &[], Some(7))
        .unwrap();
    assert!(replaced);
    assert_eq!(f.owner.composer().unwrap().queue()[0].id(), 7);
    assert_eq!(
        &*f.owner.composer().unwrap().draft().unwrap(),
        "replacement draft"
    );
    assert_eq!(
        &*f.owner.composer().unwrap().queue()[0].text(),
        "changed prompt"
    );
    assert_eq!(&*original, "original prompt");
}
