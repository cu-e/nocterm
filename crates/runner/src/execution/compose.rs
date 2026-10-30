//! Explicit composition of the native execution owners from the private runner
//! state and the administrator configuration. Nothing here is looked up
//! globally, and no API secret is an input: provider credentials arrive only as
//! original sealed commands consumed by the injected attachment owner.
use super::{ExecutionSetup, budgets::ExecutionBudgets, runtime::ExecutionRuntime};
use crate::{
    RunnerError,
    bootstrap::State,
    sessions::{NativeSessionBroker, SessionBrokerSetup, SessionCapacity, SessionResources},
};
use nocterm_agent_engine::ConfiguredProviderCatalog;
use nocterm_cloud_model::{
    admission::AdmissionLimits,
    coordinator::AuthenticatedCommandSource,
    journal::{CoordinatorSnapshotEncoder, JournalReadLimits, SnapshotLimits},
    tool::ToolTimer,
};
use nocterm_cloud_runner::{
    DurableRunJournal, DurableSnapshotPublication, NativeToolTimer,
    coordinator::{
        CoordinatorExecution, CoordinatorPublication, CoordinatorServices, CoordinatorSessions,
        CoordinatorStorage, RunCoordinator,
    },
};
use nocterm_hub_io::cloud::{
    InboxResume, RunnerArgumentFingerprint, RunnerCommandAttachments, RunnerHubPeer, RunnerInbox,
    RunnerInboxOptions, RunnerInboxPrincipal, RunnerProviderCredentialSource,
};
use nocterm_local::SystemClock;
use nocterm_ssh::{SshConfig, SshTransport};
use std::sync::Arc;

/// Staged provider/SSH answers a device may have outstanding at once.
const STAGED_COMMANDS: usize = 64;
/// Handshake/physical session caps; the per-chat cap is fixed by the broker.
const GLOBAL_SESSIONS: (usize, usize) = (16, 128);
const ACCOUNT_SESSIONS: (usize, usize) = (8, 64);

pub(super) async fn compose(
    setup: ExecutionSetup,
    state: &State,
    peer: RunnerHubPeer,
) -> Result<ExecutionRuntime, RunnerError> {
    let ExecutionSetup {
        configuration,
        targets,
        budgets,
    } = setup;
    let handle = tokio::runtime::Handle::current();
    let timer: Arc<NativeToolTimer> = Arc::new(NativeToolTimer::new(handle.clone()));
    let binding = state.connection().binding().clone();
    let grants = configuration.maximum_grants()?;
    // Containment is mandatory: a missing namespace or limit refuses startup and
    // never selects an uncontained path.
    let local = configuration.prepare_containment(state.directory()).await?;
    let ssh = SshConfig {
        known_hosts: state.directory().join("known_hosts"),
        ..SshConfig::default()
    };
    let ssh = Arc::new(SshTransport::new(ssh).map_err(RunnerError::Runtime)?);
    let destination = targets.default_destination()?;
    let targets = targets.build(binding.clone(), &local, ssh)?;

    let attachments = RunnerCommandAttachments::new(
        state.connection().clone(),
        state.material(),
        timer.clone(),
        STAGED_COMMANDS,
        budgets.attachments(),
    )?;
    let inbox = RunnerInbox::new(
        state.connection().clone(),
        state.material(),
        RunnerInboxPrincipal::new(state.account(), destination),
        Arc::new(state.journal()),
        attachments.clone(),
        timer.clone(),
        RunnerInboxOptions::new(state.directory().join("inbox"), InboxResume::OpenExisting),
    )?;
    let encoder = state.material().event_encoder(Arc::new(SystemClock));
    let journal = state
        .journal()
        .with_background_events(encoder.clone(), budgets.publication())?;
    let publication = publication(&journal, &encoder, &budgets, peer)?;
    let storage = CoordinatorStorage::new(
        binding.clone(),
        Arc::new(journal.clone()),
        Arc::new(journal.cleanup()),
        encoder,
        budgets.publication(),
        handle,
    )
    .with_publication(Arc::new(publication));
    let source: Arc<dyn AuthenticatedCommandSource> = Arc::new(inbox);
    let tool_timer: Arc<dyn ToolTimer> = timer.clone();
    let broker_setup = SessionBrokerSetup::new(
        state.connection().clone(),
        attachments.clone(),
        &storage,
        source.clone(),
        tool_timer.clone(),
        state.identity(),
        budgets.sessions(),
    )?;
    let capture = budgets.capture();
    let broker = NativeSessionBroker::new(
        broker_setup,
        targets,
        SessionResources::new(
            SessionCapacity::new(GLOBAL_SESSIONS.0, GLOBAL_SESSIONS.1, capture.clone())?,
            SessionCapacity::new(ACCOUNT_SESSIONS.0, ACCOUNT_SESSIONS.1, capture)?,
        ),
    )?;
    let credentials = RunnerProviderCredentialSource::new(attachments.clone(), state.account())?;
    let providers = ConfiguredProviderCatalog::new(
        Arc::new(credentials),
        tool_timer.clone(),
        configuration.policies(binding.scope())?,
        grants,
    )?;
    let sessions = CoordinatorSessions::new(
        binding,
        broker.clone(),
        budgets.sessions(),
        Arc::new(journal.cleanup()),
    )?;
    let execution = CoordinatorExecution::new(
        Arc::new(providers),
        broker.clone(),
        broker.clone(),
        Arc::new(RunnerArgumentFingerprint::new(budgets.arguments())),
    )
    .with_sessions(sessions);
    let services = CoordinatorServices::new(
        source,
        storage,
        execution,
        state.identity(),
        tool_timer.clone(),
    )?;
    let coordinator = RunCoordinator::with_receive_budget(
        services,
        AdmissionLimits::default(),
        budgets.receive(),
    )?;
    Ok(ExecutionRuntime::new(
        coordinator,
        broker,
        attachments,
        journal,
        tool_timer,
        budgets.transport(),
        state.limits().operation(),
    ))
}

/// The one FIFO outbox transport is the already-restored authenticated hub peer.
fn publication(
    journal: &DurableRunJournal,
    encoder: &Arc<nocterm_hub_io::cloud::RunnerEventEncoder>,
    budgets: &ExecutionBudgets,
    peer: RunnerHubPeer,
) -> Result<CoordinatorPublication, RunnerError> {
    let encoder: Arc<dyn CoordinatorSnapshotEncoder> = encoder.clone();
    let snapshots = Arc::new(DurableSnapshotPublication::new(
        journal.clone(),
        encoder,
        budgets.snapshots(),
        SnapshotLimits::new(4096, 64 * 1024 * 1024, 8 * 1024 * 1024)?,
    ));
    Ok(CoordinatorPublication::new(
        snapshots.clone(),
        snapshots.clone(),
        snapshots,
        Arc::new(peer),
        budgets.snapshots(),
        JournalReadLimits::new(256, 128 * 1024 * 1024)?,
        16,
    )?)
}
