//! Borrowed host values are copied into the original document before publication.
use super::*;
use crate::{
    history::live::LiveCopies,
    model::{ids::ModeId, tools::ToolStatus},
};

#[test]
fn copied_framing_stages_dispatch_without_a_caller_ledger() {
    let mut f = fixture();
    f.owner
        .queue_text(f.owner.local_admission(), 41, "queued text", &[], None)
        .unwrap();
    let framing = LiveCopies::new(&mut f.owner)
        .unwrap()
        .framing(
            &SessionId::new("copied session"),
            Some("copied context"),
            None,
            true,
        )
        .unwrap();
    let pending = f
        .owner
        .stage_dispatch(f.owner.local_admission(), framing, 5)
        .unwrap();
    assert_eq!(pending.payload().session().as_str(), "copied session");
    assert_eq!(&*pending.payload().context().unwrap(), "copied context");
    drop(pending);
    assert_eq!(f.owner.composer().unwrap().queue()[0].id(), 41);
}

#[test]
fn a_borrowed_display_patches_the_matching_row() {
    let mut f = fixture();
    let (entry, allocation) = tool_row(&f.ledger, "call-1").into_parts();
    let Entry::Tool(record) = entry else {
        unreachable!()
    };
    let started =
        crate::model::native::notice::tool_started(NativeOwned::from_parts(record, allocation));
    let admission = f.owner.admission(&f.epoch).unwrap();
    let _ = f.owner.apply_owned(admission, started, 1).unwrap();
    let borrowed = crate::model::retention::document::tests::display(&f.ledger, "call-1")
        .get()
        .clone();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let revision = f.owner.revision();
    let result = LiveCopies::new(&mut f.owner)
        .unwrap()
        .patch_display(admission, 1, &borrowed)
        .unwrap();
    assert!(result.saved_changed());
    assert_eq!(result.revision(), revision + 1);
    assert_eq!(tool_of(&f.owner.rows()[1]).status(), ToolStatus::Running);
    assert!(tool_of(&f.owner.rows()[1]).client_display().is_some());
}

#[test]
fn a_selected_mode_reaches_the_live_controls() {
    let mut f = fixture();
    let admission = f.owner.admission(&f.epoch).unwrap();
    LiveCopies::new(&mut f.owner)
        .unwrap()
        .mode(admission, &ModeId::new("plan"), 3)
        .unwrap();
    assert_eq!(
        f.owner
            .controls()
            .unwrap()
            .mode()
            .as_deref()
            .map(ModeId::as_str),
        Some("plan")
    );
}

#[test]
fn a_stale_admission_refuses_a_copied_selection() {
    let mut f = fixture();
    let stale = f.owner.admission(&f.epoch).unwrap();
    f.owner
        .set_draft(f.owner.local_admission(), Some("newer"))
        .unwrap();
    let revision = f.owner.revision();
    assert!(
        LiveCopies::new(&mut f.owner)
            .unwrap()
            .mode(stale, &ModeId::new("plan"), 3)
            .is_err()
    );
    assert_eq!(f.owner.revision(), revision);
}
