//! Explicit composition of the native execution owners from the private runner
//! state and the administrator configuration. Nothing here is looked up
//! globally, and no API secret is an input: provider credentials arrive only as
//! original sealed commands consumed by the injected attachment owner.
use super::{
    ExecutionSetup, budgets::ExecutionBudgets, config::ExecutionConfig, runtime::ExecutionRuntime,
};
use crate::{
    RunnerError,
    bootstrap::State,
    managed::{Approval, BrokeredProviders, CoordinatorWake},
    sessions::{
        ConfiguredSessionTargets, ContainedApproval, NativeSessionBroker, SessionBrokerSetup,
        SessionCapacity, SessionResources,
    },
};
use nocterm_agent_engine::ConfiguredProviderCatalog;
use nocterm_cloud_model::{
    admission::AdmissionLimits,
    coordinator::{AuthenticatedCommandSource, CoordinatorProviders, RunDestination},
    journal::{
        CoordinatorSnapshotEncoder, JournalCatalogBinding, JournalReadLimits, SnapshotLimits,
    },
    tool::ToolTimer,
};
use nocterm_cloud_runner::{
    DurableRunJournal, DurableSnapshotPublication, NativeToolTimer,
    coordinator::{
        CoordinatorExecution, CoordinatorPublication, CoordinatorServices, CoordinatorSessions,
        CoordinatorStorage, RunCoordinator, ToolApprovalPolicy,
    },
};
use nocterm_hub_io::cloud::{
    InboxResume, RunnerArgumentFingerprint, RunnerChatRetirement, RunnerCommandAttachments,
    RunnerHubPeer, RunnerInbox, RunnerInboxOptions, RunnerInboxPrincipal,
    RunnerProviderCredentialSource,
};
use nocterm_local::SystemClock;
use nocterm_ssh::{SshConfig, SshTransport};
use std::sync::Arc;

/// Staged provider/SSH answers a device may have outstanding at once.
const STAGED_COMMANDS: usize = 64;
/// Handshake/physical session caps; the per-chat cap is fixed by the broker.
const GLOBAL_SESSIONS: (usize, usize) = (16, 128);
const ACCOUNT_SESSIONS: (usize, usize) = (8, 64);

/// Host keys are pinned in the private state directory; no agent or identity
/// directory is consulted.
fn ssh_transport(state: &State) -> Result<Arc<SshTransport>, RunnerError> {
    let config = SshConfig {
        known_hosts: state.directory().join("known_hosts"),
        ..SshConfig::default()
    };
    Ok(Arc::new(
        SshTransport::new(config).map_err(RunnerError::Runtime)?,
    ))
}

fn ingress(
    state: &State,
    timer: &Arc<NativeToolTimer>,
    budgets: &ExecutionBudgets,
    destination: RunDestination,
) -> Result<(Arc<RunnerCommandAttachments>, RunnerInbox), RunnerError> {
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
    Ok((attachments, inbox))
}

fn broker(
    setup: SessionBrokerSetup,
    targets: Arc<ConfiguredSessionTargets>,
    budgets: &ExecutionBudgets,
) -> Result<Arc<NativeSessionBroker>, RunnerError> {
    let capture = budgets.capture();
    Ok(NativeSessionBroker::new(
        setup,
        targets,
        SessionResources::new(
            SessionCapacity::new(GLOBAL_SESSIONS.0, GLOBAL_SESSIONS.1, capture.clone())?,
            SessionCapacity::new(ACCOUNT_SESSIONS.0, ACCOUNT_SESSIONS.1, capture)?,
        ),
    )?)
}

/// A managed runner calls the host broker with per-chat tokens; every other
/// runner installs the original sealed provider commands of its devices.
fn providers(
    configuration: ExecutionConfig,
    brokered: Option<&Arc<BrokeredProviders>>,
    attachments: &Arc<RunnerCommandAttachments>,
    state: &State,
    timer: &Arc<dyn ToolTimer>,
    grants: usize,
) -> Result<Arc<dyn CoordinatorProviders>, RunnerError> {
    if let Some(brokered) = brokered {
        return Ok(brokered.clone());
    }
    let credentials = RunnerProviderCredentialSource::new(attachments.clone(), state.account())?;
    let policies = configuration.policies(state.connection().binding().scope())?;
    Ok(Arc::new(ConfiguredProviderCatalog::new(
        Arc::new(credentials),
        timer.clone(),
        policies,
        grants,
    )?))
}

/// `approval: "never"` trusts the runner's own containment; every other mode
/// keeps the coordinator's default of asking for each mutating call (ADR 0127).
fn approval_policy(
    brokered: Option<&Arc<BrokeredProviders>>,
    targets: &ConfiguredSessionTargets,
) -> Option<Arc<dyn ToolApprovalPolicy>> {
    let brokered = brokered?;
    (brokered.policy().approval() == Approval::Never)
        .then(|| Arc::new(ContainedApproval::new(targets.contained_targets())) as Arc<_>)
}

/// A token issued after a prompt parked resumes it without another command.
fn bind_wake(brokered: Option<&Arc<BrokeredProviders>>, coordinator: &RunCoordinator) {
    if let Some(brokered) = brokered {
        brokered.tokens().bind_wake(Arc::new(CoordinatorWake::new(
            coordinator.clone(),
            brokered.provider().clone(),
        )));
    }
}

pub(super) async fn compose(
    setup: ExecutionSetup,
    state: &State,
    peer: RunnerHubPeer,
    brokered: Option<Arc<BrokeredProviders>>,
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
    let ssh = ssh_transport(state)?;
    let destination = targets.default_destination()?;
    let targets = targets.build(binding.clone(), &local, ssh)?;
    let approval = approval_policy(brokered.as_ref(), &targets);

    let (attachments, inbox) = ingress(state, &timer, &budgets, destination)?;
    let encoder = state.material().event_encoder(Arc::new(SystemClock));
    let journal = state
        .journal()
        .with_background_events(encoder.clone(), budgets.publication())?;
    let publication = publication(&journal, &encoder, &budgets, peer.clone())?;
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
    let broker = broker(broker_setup, targets, &budgets)?;
    let providers = providers(
        configuration,
        brokered.as_ref(),
        &attachments,
        state,
        &tool_timer,
        grants,
    )?;
    let retirement = Arc::new(RunnerChatRetirement::new(
        peer.clone(),
        state.material(),
        tokio::runtime::Handle::current(),
    ));
    let execution = execution(binding, providers, &broker, &journal, &budgets, approval)?
        .with_retirement(retirement);
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
    bind_wake(brokered.as_ref(), &coordinator);
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

fn execution(
    binding: JournalCatalogBinding,
    providers: Arc<dyn CoordinatorProviders>,
    broker: &Arc<NativeSessionBroker>,
    journal: &DurableRunJournal,
    budgets: &ExecutionBudgets,
    approval: Option<Arc<dyn ToolApprovalPolicy>>,
) -> Result<CoordinatorExecution, RunnerError> {
    let sessions = CoordinatorSessions::new(
        binding,
        broker.clone(),
        budgets.sessions(),
        Arc::new(journal.cleanup()),
    )?;
    let execution = CoordinatorExecution::new(
        providers,
        broker.clone(),
        broker.clone(),
        Arc::new(RunnerArgumentFingerprint::new(budgets.arguments())),
    )
    .with_sessions(sessions);
    Ok(match approval {
        Some(policy) => execution.with_approval_policy(policy),
        None => execution,
    })
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
