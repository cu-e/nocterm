//! Chat summaries reach the runner directory from the delivery sweep (ADR 0135).
use super::*;

const DATABASE: &str = "cloud-runs.sqlite3";

async fn prompted(fixture: &Fixture) -> RunBinding {
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    fixture.binding().await
}
fn revisions(fixture: &Fixture) -> Vec<u64> {
    fixture
        .directory
        .published()
        .iter()
        .map(SealedChatSummary::revision)
        .collect()
}
fn last_draft(fixture: &Fixture) -> ChatSummaryDraft {
    fixture.directory.drafts().pop().unwrap()
}

#[tokio::test]
async fn a_prompted_chat_publishes_a_summary_named_by_its_first_prompt() {
    let fixture = Fixture::new();
    prompted(&fixture).await;
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1]);
    let draft = last_draft(&fixture);
    assert_eq!(draft.prompt_line(), Some("original prompt"));
    assert_eq!(draft.model().unwrap().model().as_str(), "model");
    assert_eq!(draft.activity(), ChatActivity::Running);
    assert_eq!(draft.created_at(), Timestamp::from_millis(1000));
    let stored = fixture.journal.summary(stream()).await.unwrap().unwrap();
    assert_eq!((stored.revision(), stored.published()), (1, 1));
    assert_eq!(stored.pending(), None);
}

#[tokio::test]
async fn summaries_are_coalesced_to_one_per_interval_and_follow_later_changes() {
    let fixture = Fixture::new();
    let run = prompted(&fixture).await;
    fixture.actor.publish_summary(stream()).await.unwrap();
    let stop = command(2, fixture.control().await, Some(run), CommandBody::Stop);
    fixture.actor.accept(stop).await.unwrap();
    fixture.timer.set(5999);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1]);
    fixture.timer.set(6000);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1, 2]);
    let draft = last_draft(&fixture);
    assert_eq!(draft.activity(), ChatActivity::Idle);
    assert_eq!(draft.prompt_line(), Some("original prompt"));
    assert_eq!(draft.last_activity(), Timestamp::from_millis(5999));
    fixture.timer.set(60_000);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1, 2]);
}

#[tokio::test]
async fn a_deletion_is_published_at_once_and_ends_the_chat_summaries() {
    let fixture = Fixture::new();
    prompted(&fixture).await;
    fixture.actor.publish_summary(stream()).await.unwrap();
    let deletion = command(2, fixture.control().await, None, CommandBody::DeleteChat);
    fixture.actor.accept(deletion).await.unwrap();
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1, 2]);
    assert_eq!(last_draft(&fixture).activity(), ChatActivity::Deleted);
    assert!(fixture.directory.published()[1].deleted());
    fixture.timer.set(600_000);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1, 2]);
}

#[tokio::test]
async fn a_failed_publication_is_retried_later_with_the_same_sealed_bytes() {
    let fixture = Fixture::new();
    prompted(&fixture).await;
    fixture.directory.fail_next(1);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert!(fixture.directory.published().is_empty());
    let staged = fixture.journal.summary(stream()).await.unwrap().unwrap();
    let pending = staged.pending().unwrap().clone();
    fixture.timer.set(10_999);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert!(fixture.directory.published().is_empty());
    fixture.timer.set(11_000);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(fixture.directory.published(), [pending]);
    assert_eq!(fixture.directory.drafts().len(), 1);
}

#[tokio::test]
async fn nothing_is_sealed_while_the_hub_keeps_no_runner_directory() {
    let fixture = Fixture::new();
    prompted(&fixture).await;
    fixture.directory.set_available(false);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert!(fixture.directory.drafts().is_empty());
    assert_eq!(fixture.journal.summary(stream()).await.unwrap(), None);
    fixture.directory.set_available(true);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1]);
}

#[tokio::test]
async fn an_upgraded_version_6_journal_backfills_titles_from_its_records() {
    let mut fixture = Fixture::new();
    prompted(&fixture).await;
    fixture.actor.fence();
    let db = rusqlite::Connection::open(fixture.root.path().join(DATABASE)).unwrap();
    db.execute_batch("DROP TABLE chat_summaries; DROP TABLE chat_ssh_keys; PRAGMA user_version=6")
        .unwrap();
    drop(db);
    fixture.restart();
    fixture.timer.set(90_000);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1]);
    let draft = last_draft(&fixture);
    assert_eq!(draft.prompt_line(), Some("original prompt"));
    assert_eq!(draft.created_at(), Timestamp::from_millis(1000));
}

#[tokio::test]
async fn a_restarted_runner_continues_the_revisions_it_published() {
    let mut fixture = Fixture::new();
    prompted(&fixture).await;
    fixture.actor.publish_summary(stream()).await.unwrap();
    fixture.restart();
    fixture.timer.set(2000);
    fixture.actor.publish_summary(stream()).await.unwrap();
    assert_eq!(revisions(&fixture), [1, 2]);
    assert_eq!(
        last_draft(&fixture).created_at(),
        Timestamp::from_millis(1000)
    );
}
