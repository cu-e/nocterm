//! One prompt and its reply through an opened Cloud chat, without a window.
//!
//! The chat view drives the same captured source this way: it prepares and stages the command in
//! the encrypted outbox, delivers it, and replays the runner's verified pages into a projection.
//! The test is that view with its allocations taken from a plain receive budget.
use nocterm_ai::{conversation::ConversationAction, thread::Entry};
use nocterm_cloud_model::coordinator::{CommandReceiveBudget, IntakeLane, IntakeLimits};
use nocterm_cloud_model::projection::{
    ConversationProjection, MAX_REPLAY_BYTES, ProjectionBatch,
    ports::{
        CapturedConversationSource, CommandDelivery, ConversationCommand, ConversationCoordinates,
        ConversationReplay, ConversationWork,
    },
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Waits for `step` for at most `patience`; a step the hub never answers fails the test.
fn within<T>(what: &str, patience: Duration, step: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::time::timeout(patience, step).await })
        .unwrap_or_else(|_| panic!("{what} did not finish within {patience:?}"))
}

fn lease(budget: &CommandReceiveBudget, bytes: usize, payloads: usize) -> AllocationLease {
    let reserved = budget
        .try_reserve(IntakeLane::Ordinary, bytes, payloads)
        .expect("the receive budget has room for one chat exchange");
    AllocationLease::new(bytes, payloads, reserved)
}

/// Sends `prompt` and returns the assistant's reply once the runner has published one, or
/// panics with the transcript when none arrives within `patience`.
pub(crate) fn reply_to(
    source: &Arc<dyn CapturedConversationSource>,
    prompt: &str,
    patience: Duration,
) -> String {
    let budget = CommandReceiveBudget::new(IntakeLimits::default());
    let mut projection = ConversationProjection::new(source.binding().clone());
    let coordinates = ConversationCoordinates {
        binding: projection.binding().clone(),
        run: None,
        control: projection.control(),
    };
    let command: ConversationCommand = source
        .prepare(
            coordinates,
            ConversationAction::Prompt(prompt.to_owned()).into(),
        )
        .expect("the prompt is prepared");
    let id = command.id;
    let stage = source
        .allocation_requirements(ConversationWork::Stage { command: &command })
        .unwrap();
    let held = lease(&budget, command.resident_bytes().saturating_add(4096), 1);
    let command = Arc::new(Allocated::new(command, held));
    within(
        "staging the prompt",
        patience,
        source.stage(command, lease(&budget, stage.bytes(), stage.payloads())),
    )
    .expect("the prompt is committed to the outbox");
    let delivered = within(
        "delivering the prompt",
        patience,
        source.deliver(id, lease(&budget, stage.bytes(), stage.payloads())),
    )
    .expect("the prompt is delivered");
    assert_ne!(
        delivered,
        CommandDelivery::NotFound,
        "the hub never saw the prompt"
    );

    let deadline = Instant::now() + patience;
    while Instant::now() < deadline {
        let need = source
            .allocation_requirements(ConversationWork::Replay {
                after: projection.cursor(),
                maximum_bytes: MAX_REPLAY_BYTES,
            })
            .unwrap();
        let work = lease(&budget, need.bytes(), need.payloads());
        if let Some(page) = next_page(source, &projection, work) {
            projection.apply(page.value()).expect("the page applies");
        }
        let reply = assistant_text(&projection);
        if !reply.trim().is_empty() {
            return reply;
        }
    }
    panic!(
        "no reply within {patience:?}; the chat is {:?} with {:?}",
        projection.phase(),
        projection.transcript().entries
    );
}

/// The next verified page, or `None` when the runner published nothing within a short wait.
fn next_page(
    source: &Arc<dyn CapturedConversationSource>,
    projection: &ConversationProjection,
    work: AllocationLease,
) -> Option<Box<Allocated<ProjectionBatch>>> {
    let page = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(
                Duration::from_secs(10),
                source.next(projection.cursor(), work),
            )
            .await
        })
        .ok()?;
    match page.expect("a verified page") {
        ConversationReplay::Page(batch) => Some(batch),
        _ => None,
    }
}

fn assistant_text(projection: &ConversationProjection) -> String {
    projection
        .transcript()
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Agent(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
