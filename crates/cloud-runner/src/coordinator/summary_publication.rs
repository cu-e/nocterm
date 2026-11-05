//! Publishes each chat's summary to the runner's directory stream (ADR 0135).
//!
//! The runner's delivery sweep calls [`publish`] for every catalog chat. The first call for
//! a chat derives its facts from the whole journal, which also backfills chats that existed
//! before summaries; later calls read only the records after the last one seen. A due
//! revision is sealed by the directory, staged in the journal and then published; failures
//! wait and retry the staged bytes. All journal work runs through the coordinator's one
//! native journal gate, and network waits never hold it.
use super::{
    frame::Frame, recovery, replay::JournalReplay, services::CoordinatorServices,
    supervisor::TaskClass,
};
use nocterm_cloud_model::{
    authority::StreamBinding,
    coordinator::{CommandBody, CoordinatorError, RunStatus},
    identity::{JournalSequence, Timestamp},
    journal::*,
    run::RunPhase,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex as AsyncMutex;

/// Chats one coordinator follows, as many as it keeps open.
const MAX_CHATS: usize = 4096;

/// The directory and summary rows a runner publishes with.
pub struct CoordinatorSummaries {
    journal: Arc<dyn ChatSummaryJournal>,
    directory: Arc<dyn ChatSummaryDirectory>,
    chats: AsyncMutex<BTreeMap<StreamBinding, Slot>>,
}
impl CoordinatorSummaries {
    pub fn new(
        journal: Arc<dyn ChatSummaryJournal>,
        directory: Arc<dyn ChatSummaryDirectory>,
    ) -> Self {
        Self {
            journal,
            directory,
            chats: AsyncMutex::new(BTreeMap::new()),
        }
    }
}

enum Slot {
    Tracked {
        tracker: Box<ChatSummaryTracker>,
        through: JournalSequence,
    },
    /// Loading failed; the chat is derived again from its journal at `until`.
    Waiting { until: Timestamp },
    /// Nothing more is published: the deletion is out, or the chat was deleted before
    /// summaries existed or its records are no longer readable.
    Retired,
}

pub(super) async fn publish(
    services: Arc<CoordinatorServices>,
    stream: StreamBinding,
) -> Result<(), CoordinatorError> {
    let Some(owner) = services.summaries().cloned() else {
        return Ok(());
    };
    let tasks = services.tasks().clone();
    tasks
        .submit(
            TaskClass::Native,
            Box::pin(async move {
                let mut chats = owner.chats.lock().await;
                services.source().check_current()?;
                let now = services.timer().now();
                let reload = match chats.get(&stream) {
                    None if chats.len() >= MAX_CHATS => return Err(CoordinatorError::Overloaded),
                    None => true,
                    Some(Slot::Waiting { until }) => now >= *until,
                    Some(_) => false,
                };
                if reload {
                    match load(&services, &owner, &stream).await {
                        Ok(slot) => chats.insert(stream.clone(), slot),
                        Err(error) => {
                            chats.insert(stream, waiting(now, RELOAD_WAIT_MS));
                            return Err(error);
                        }
                    };
                }
                let Some(Slot::Tracked { tracker, through }) = chats.get_mut(&stream) else {
                    return Ok(());
                };
                let result = advance(&services, &owner, tracker, through).await;
                match &result {
                    // The stored row moved past this owner: derive the chat again now.
                    Err(CoordinatorError::Journal(JournalError::Conflict)) => {
                        chats.insert(stream, waiting(now, 0));
                    }
                    Ok(()) if tracker.finished() => {
                        chats.insert(stream, Slot::Retired);
                    }
                    // Other failures keep the tracker; the next sweep continues from it.
                    _ => {}
                }
                result
            }),
        )?
        .await
}

/// A chat whose derivation failed (a journal over the replay bound, a busy store) is
/// derived again after this wait rather than at every sweep.
const RELOAD_WAIT_MS: u64 = 60_000;
fn waiting(now: Timestamp, wait: u64) -> Slot {
    Slot::Waiting {
        until: Timestamp::from_millis(now.as_millis().saturating_add(wait)),
    }
}

async fn load(
    services: &CoordinatorServices,
    owner: &CoordinatorSummaries,
    stream: &StreamBinding,
) -> Result<Slot, CoordinatorError> {
    let journal = owner.journal.clone();
    let lookup = stream.clone();
    let stored = services.io().run(move |_| journal.summary(lookup)).await?;
    let lookup = stream.clone();
    let head = match services.io().run(move |journal| journal.head(lookup)).await {
        Ok(head) => head.ok_or(JournalError::NotFound)?,
        Err(CoordinatorError::Journal(JournalError::KeyUnavailable | JournalError::Fenced)) => {
            return Ok(Slot::Retired);
        }
        Err(error) => return Err(error),
    };
    if head.deleted()
        && stored
            .as_ref()
            .is_none_or(|stored| stored.deleted() && stored.pending().is_none())
    {
        return Ok(Slot::Retired);
    }
    let now = services.timer().now();
    let (facts, through) = match JournalReplay::load(services, stream.clone()).await {
        Ok(replay) => derive(replay.records(), now)?,
        Err(CoordinatorError::Journal(JournalError::KeyUnavailable | JournalError::Fenced))
            if head.deleted() =>
        {
            (ChatSummaryFacts::deleted_at(now), head.position().records())
        }
        Err(error) => return Err(error),
    };
    Ok(Slot::Tracked {
        tracker: Box::new(ChatSummaryTracker::restore(stream.clone(), facts, stored)?),
        through,
    })
}

/// Facts of a whole journal. Records carry no time of their own: a command counts at its
/// issue time and every later record at the latest time seen.
fn derive(
    records: &[StoredRecord],
    fallback: Timestamp,
) -> Result<(ChatSummaryFacts, JournalSequence), CoordinatorError> {
    let mut facts = ChatSummaryFacts::default();
    let mut latest: Option<Timestamp> = None;
    for stored in records {
        observe(stored, |observation, issued| {
            let at = issued.or(latest).unwrap_or(fallback);
            latest = Some(latest.map_or(at, |latest| latest.max(at)));
            facts.observe(observation, at);
        })?;
    }
    let through = records
        .last()
        .map_or(JournalSequence::default(), StoredRecord::sequence);
    Ok((facts, through))
}

/// Applies the records after `through` at the current time, then publishes what is due.
async fn advance(
    services: &CoordinatorServices,
    owner: &CoordinatorSummaries,
    tracker: &mut ChatSummaryTracker,
    through: &mut JournalSequence,
) -> Result<(), CoordinatorError> {
    let stream = tracker.stream().clone();
    loop {
        let lookup = stream.clone();
        let after = *through;
        let limits = JournalReadLimits::new(64, 16 * 1024 * 1024)?;
        let page = match services
            .io()
            .run(move |journal| journal.read(lookup, after, limits))
            .await
        {
            Ok(page) => page,
            Err(CoordinatorError::Journal(JournalError::Gap { .. })) => {
                // A compaction removed records this owner had not seen yet.
                let replay = JournalReplay::load(services, stream.clone()).await?;
                let (facts, last) = derive(replay.records(), services.timer().now())?;
                tracker.reload(facts);
                *through = last;
                break;
            }
            Err(error) => return Err(error),
        };
        let now = services.timer().now();
        for stored in page.records() {
            observe(stored, |observation, _| tracker.observe(observation, now))?;
        }
        *through = page.next();
        if page.records().is_empty() || *through >= page.head().records() {
            break;
        }
    }
    if let Some(ssh) = services.ssh() {
        tracker.observe_ssh(ssh.summary(&stream));
    }
    deliver(services, owner, tracker).await
}

async fn deliver(
    services: &CoordinatorServices,
    owner: &CoordinatorSummaries,
    tracker: &mut ChatSummaryTracker,
) -> Result<(), CoordinatorError> {
    let now = services.timer().now();
    let step = tracker.next(now);
    if matches!(step, SummaryStep::Idle) || !owner.directory.available() {
        return Ok(());
    }
    let sealed = match step {
        SummaryStep::Idle => return Ok(()),
        SummaryStep::Publish(sealed) => sealed,
        SummaryStep::Seal(draft) => {
            let (created_at, last_activity) = (draft.created_at(), draft.last_activity());
            let Ok(sealed) = owner.directory.seal(draft).await else {
                tracker.failed(now);
                return Ok(());
            };
            services.source().check_current()?;
            let journal = owner.journal.clone();
            let staged = sealed.clone();
            services
                .io()
                .run(move |_| journal.stage_summary(staged, created_at, last_activity))
                .await?;
            tracker.staged(sealed.clone())?;
            sealed
        }
    };
    let (stream, revision) = (sealed.stream().clone(), sealed.revision());
    if owner.directory.publish(sealed).await.is_err() {
        // The directory reports its own causes; the staged bytes are retried later.
        tracker.failed(now);
        return Ok(());
    }
    services.source().check_current()?;
    let journal = owner.journal.clone();
    services
        .io()
        .run(move |_| journal.summary_published(stream, revision))
        .await?;
    tracker.published(revision, services.timer().now());
    Ok(())
}

/// Maps one native record to what a summary needs. `sink` receives the observation and,
/// for a command, its issue time.
fn observe(
    stored: &StoredRecord,
    mut sink: impl FnMut(SummaryObservation<'_>, Option<Timestamp>),
) -> Result<(), CoordinatorError> {
    let record = stored.record();
    let run = record.context().run();
    match (record.kind(), run) {
        (JournalRecordKind::CommandAccepted { .. }, _) => {
            let Frame::Accepted { command, allocated } = Frame::read(record.payload())? else {
                return Err(JournalError::Corrupt.into());
            };
            let command = command.restore()?;
            let issued = Some(command.window().issued());
            match command.body() {
                CommandBody::Submit(request) => {
                    let run = allocated.ok_or(JournalError::Corrupt)?.restore()?;
                    let observation = SummaryObservation::Prompt {
                        run: &run,
                        prompt: request.prompt(),
                        model: request.selection(),
                    };
                    sink(observation, issued);
                }
                CommandBody::DeleteChat => sink(SummaryObservation::Deleted, issued),
                _ => sink(SummaryObservation::Activity, issued),
            }
        }
        (JournalRecordKind::RunStatus, Some(run)) => match Frame::read(record.payload())? {
            Frame::Status(tag) => sink(
                SummaryObservation::Status {
                    run,
                    status: recovery::status(tag)?,
                },
                None,
            ),
            _ => sink(SummaryObservation::Activity, None),
        },
        (JournalRecordKind::Phase(phase), Some(run)) => sink(
            SummaryObservation::Status {
                run,
                status: phase_status(*phase),
            },
            None,
        ),
        (JournalRecordKind::Approval { intent, open }, Some(run)) => sink(
            SummaryObservation::Approval {
                run,
                intent: *intent,
                open: *open,
            },
            None,
        ),
        (JournalRecordKind::Tombstone { .. }, _) => sink(SummaryObservation::Deleted, None),
        _ => sink(SummaryObservation::Activity, None),
    }
    Ok(())
}

fn phase_status(phase: RunPhase) -> RunStatus {
    match phase {
        RunPhase::Running => RunStatus::Running,
        RunPhase::OutcomeUnknown => RunStatus::OutcomeUnknown,
        RunPhase::Interrupted => RunStatus::Interrupted,
        RunPhase::Completed => RunStatus::Completed,
    }
}
