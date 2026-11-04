use super::*;
use crate::identity::{
    AccountIdentity, AccountScope, CloudChatId, Fingerprint, HubIdentity, NamespaceId, RunEpoch,
    RunId, RunnerId, RunnerKeyEpoch,
};

fn run(id: u8) -> RunBinding {
    RunBinding::new(
        AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("hub").unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new("account").unwrap(),
                Fingerprint::from_bytes([2; 32]),
            ),
        ),
        RunnerId::from_bytes([3; 16]),
        RunnerKeyEpoch::new(1).unwrap(),
        CloudChatId::from_bytes([4; 16]),
        RunId::from_bytes([id; 16]),
        RunEpoch::new(1).unwrap(),
    )
}
fn model(name: &str) -> ModelSelection {
    ModelSelection::new(
        NamespaceId::new("provider").unwrap(),
        NamespaceId::new(name).unwrap(),
        NamespaceId::new("pricing").unwrap(),
    )
}
fn at(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}
fn prompted(facts: &mut ChatSummaryFacts, id: u8, text: &str, name: &str, ms: u64) {
    let prompt = EnginePrompt::new(vec![text.into()]).unwrap();
    facts.observe(
        SummaryObservation::Prompt {
            run: &run(id),
            prompt: &prompt,
            model: &model(name),
        },
        at(ms),
    );
}
fn status(facts: &mut ChatSummaryFacts, id: u8, status: RunStatus, ms: u64) {
    facts.observe(
        SummaryObservation::Status {
            run: &run(id),
            status,
        },
        at(ms),
    );
}
fn sealed(revision: u64, deleted: bool) -> SealedChatSummary {
    SealedChatSummary::new(
        run(1).stream(),
        ChatKeyEpoch::new(1).unwrap(),
        revision,
        deleted,
        OpaqueBytes::new(vec![revision as u8; 32]).unwrap(),
    )
    .unwrap()
}
fn seal_next(tracker: &mut ChatSummaryTracker, now: u64) -> ChatSummaryDraft {
    let SummaryStep::Seal(draft) = tracker.next(at(now)) else {
        panic!("a new revision is due");
    };
    tracker
        .staged(sealed(
            draft.revision(),
            draft.activity() == ChatActivity::Deleted,
        ))
        .unwrap();
    draft
}

#[test]
fn the_first_accepted_prompt_names_the_chat_and_the_latest_prompt_names_the_model() {
    let mut facts = ChatSummaryFacts::default();
    prompted(
        &mut facts,
        1,
        "\n\n   Fix the build  \nmore text",
        "first",
        10,
    );
    prompted(&mut facts, 2, "Another question", "second", 20);
    assert_eq!(facts.prompt_line(), Some("Fix the build"));
    assert_eq!(facts.model().unwrap().model().as_str(), "second");
    assert_eq!(facts.created_at(), Some(at(10)));
    assert_eq!(facts.last_activity(), Some(at(20)));
}

#[test]
fn a_long_first_line_is_kept_to_its_bound_on_a_character_boundary() {
    let mut facts = ChatSummaryFacts::default();
    prompted(&mut facts, 1, &"é".repeat(400), "model", 1);
    let line = facts.prompt_line().unwrap();
    assert!(line.len() <= MAX_SUMMARY_PROMPT_LINE_BYTES);
    assert!(line.len() >= MAX_SUMMARY_PROMPT_LINE_BYTES - 1);
    assert!(line.chars().all(|character| character == 'é'));
}

#[test]
fn activity_follows_the_most_demanding_unfinished_run_and_the_last_outcome() {
    let mut facts = ChatSummaryFacts::default();
    assert_eq!(facts.activity(), ChatActivity::Idle);
    prompted(&mut facts, 1, "one", "model", 1);
    assert_eq!(facts.activity(), ChatActivity::Running);
    prompted(&mut facts, 2, "two", "model", 2);
    status(&mut facts, 2, RunStatus::NeedsProviderAccess, 3);
    assert_eq!(facts.activity(), ChatActivity::NeedsProvider);
    status(&mut facts, 1, RunStatus::NeedsUnlock, 4);
    assert_eq!(facts.activity(), ChatActivity::NeedsProvider);
    status(&mut facts, 2, RunStatus::Completed, 5);
    assert_eq!(facts.activity(), ChatActivity::NeedsSsh);
    status(&mut facts, 1, RunStatus::Interrupted, 6);
    assert_eq!(facts.activity(), ChatActivity::Failed);
    prompted(&mut facts, 3, "three", "model", 7);
    status(&mut facts, 3, RunStatus::Completed, 8);
    assert_eq!(facts.activity(), ChatActivity::Idle);
}

#[test]
fn open_approvals_are_counted_until_answered_or_their_run_ends() {
    let mut facts = ChatSummaryFacts::default();
    prompted(&mut facts, 1, "one", "model", 1);
    for (intent, open) in [(1, true), (2, true), (1, false)] {
        facts.observe(
            SummaryObservation::Approval {
                run: &run(1),
                intent: IntentId::from_bytes([intent; 16]),
                open,
            },
            at(2),
        );
    }
    assert_eq!(facts.pending_approvals(), 1);
    assert_eq!(facts.activity(), ChatActivity::WaitingApproval);
    status(&mut facts, 1, RunStatus::Stopped, 3);
    assert_eq!(facts.pending_approvals(), 0);
    assert_eq!(facts.activity(), ChatActivity::Idle);
}

#[test]
fn a_deletion_is_final_for_the_facts() {
    let mut facts = ChatSummaryFacts::default();
    prompted(&mut facts, 1, "one", "model", 1);
    facts.observe(SummaryObservation::Deleted, at(2));
    prompted(&mut facts, 2, "two", "other", 3);
    assert_eq!(facts.activity(), ChatActivity::Deleted);
    assert_eq!(facts.model().unwrap().model().as_str(), "model");
    assert_eq!(facts.last_activity(), Some(at(3)));
}

#[test]
fn a_backfilled_chat_publishes_its_first_revision_at_once_then_at_most_every_interval() {
    let mut facts = ChatSummaryFacts::default();
    prompted(&mut facts, 1, "hello", "model", 100);
    let mut tracker = ChatSummaryTracker::restore(run(1).stream(), facts, None).unwrap();
    let draft = seal_next(&mut tracker, 1_000);
    assert_eq!(draft.revision(), 1);
    assert_eq!(draft.prompt_line(), Some("hello"));
    assert_eq!(draft.created_at(), at(100));
    let SummaryStep::Publish(first) = tracker.next(at(1_000)) else {
        panic!("the staged revision is published");
    };
    assert_eq!(first.revision(), 1);
    tracker.published(1, at(1_000));
    assert!(matches!(tracker.next(at(1_001)), SummaryStep::Idle));
    tracker.observe(SummaryObservation::Activity, at(1_002));
    assert!(matches!(tracker.next(at(5_999)), SummaryStep::Idle));
    assert_eq!(seal_next(&mut tracker, 6_000).revision(), 2);
}

#[test]
fn a_failed_publication_retries_the_same_bytes_with_a_growing_wait() {
    let mut tracker =
        ChatSummaryTracker::restore(run(1).stream(), ChatSummaryFacts::default(), None).unwrap();
    seal_next(&mut tracker, 0);
    tracker.failed(at(0));
    assert!(matches!(tracker.next(at(9_999)), SummaryStep::Idle));
    let SummaryStep::Publish(retry) = tracker.next(at(10_000)) else {
        panic!("the same revision is retried");
    };
    assert_eq!(retry, sealed(1, false));
    tracker.failed(at(10_000));
    assert!(matches!(tracker.next(at(29_999)), SummaryStep::Idle));
    assert!(matches!(tracker.next(at(30_000)), SummaryStep::Publish(_)));
}

#[test]
fn facts_that_overtake_a_staged_revision_replace_it_with_a_higher_one() {
    let mut tracker =
        ChatSummaryTracker::restore(run(1).stream(), ChatSummaryFacts::default(), None).unwrap();
    seal_next(&mut tracker, 0);
    tracker.failed(at(0));
    tracker.observe(SummaryObservation::Activity, at(1));
    assert_eq!(seal_next(&mut tracker, 10_000).revision(), 2);
    tracker.published(1, at(10_000));
    let SummaryStep::Publish(newest) = tracker.next(at(20_000)) else {
        panic!("an older confirmation keeps the newer revision pending");
    };
    assert_eq!(newest.revision(), 2);
}

#[test]
fn a_deletion_is_published_at_once_and_then_nothing_follows() {
    let mut tracker =
        ChatSummaryTracker::restore(run(1).stream(), ChatSummaryFacts::default(), None).unwrap();
    seal_next(&mut tracker, 0);
    tracker.published(1, at(0));
    tracker.observe(SummaryObservation::Deleted, at(1));
    let draft = seal_next(&mut tracker, 1);
    assert_eq!(draft.activity(), ChatActivity::Deleted);
    tracker.published(2, at(1));
    assert!(tracker.finished());
    tracker.observe(SummaryObservation::Activity, at(2));
    assert!(matches!(tracker.next(at(60_000)), SummaryStep::Idle));
}

#[test]
fn a_restored_row_keeps_its_times_and_revision_and_republishes_once() {
    let mut facts = ChatSummaryFacts::default();
    prompted(&mut facts, 1, "hello", "model", 50);
    let stored = StoredChatSummary::new(4, 4, at(20), at(900), false, None).unwrap();
    let mut tracker = ChatSummaryTracker::restore(run(1).stream(), facts, Some(stored)).unwrap();
    let draft = seal_next(&mut tracker, 1_000);
    assert_eq!(draft.revision(), 5);
    assert_eq!(draft.created_at(), at(20));
    assert_eq!(draft.last_activity(), at(900));
}

#[test]
fn a_restored_published_deletion_stays_final() {
    let stored = StoredChatSummary::new(3, 3, at(1), at(2), true, None).unwrap();
    let tracker = ChatSummaryTracker::restore(
        run(1).stream(),
        ChatSummaryFacts::deleted_at(at(2)),
        Some(stored),
    )
    .unwrap();
    assert!(tracker.finished());
    assert!(matches!(tracker.next(at(0)), SummaryStep::Idle));
}

#[test]
fn a_reload_after_compaction_keeps_the_title_model_and_times() {
    let mut facts = ChatSummaryFacts::default();
    prompted(&mut facts, 1, "hello", "model", 50);
    let mut tracker = ChatSummaryTracker::restore(run(1).stream(), facts, None).unwrap();
    let mut later = ChatSummaryFacts::default();
    status(&mut later, 1, RunStatus::Running, 70);
    tracker.reload(later);
    assert_eq!(tracker.facts().prompt_line(), Some("hello"));
    assert_eq!(tracker.facts().created_at(), Some(at(50)));
    assert_eq!(tracker.facts().last_activity(), Some(at(70)));
    assert_eq!(tracker.facts().activity(), ChatActivity::Running);
}

#[test]
fn stored_rows_and_staging_refuse_inconsistent_revisions() {
    assert!(matches!(
        StoredChatSummary::new(2, 1, at(1), at(1), false, None),
        Err(JournalError::Corrupt)
    ));
    assert!(matches!(
        StoredChatSummary::new(2, 1, at(1), at(1), false, Some(sealed(1, false))),
        Err(JournalError::Corrupt)
    ));
    assert!(matches!(
        StoredChatSummary::new(1, 1, at(5), at(4), false, None),
        Err(JournalError::Corrupt)
    ));
    assert!(matches!(
        SealedChatSummary::new(
            run(1).stream(),
            ChatKeyEpoch::new(1).unwrap(),
            0,
            false,
            OpaqueBytes::new(vec![1]).unwrap()
        ),
        Err(JournalError::Capacity)
    ));
    let mut tracker =
        ChatSummaryTracker::restore(run(1).stream(), ChatSummaryFacts::default(), None).unwrap();
    assert!(matches!(
        tracker.staged(sealed(2, false)),
        Err(JournalError::Conflict)
    ));
}
