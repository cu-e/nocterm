//! Explicit scripted ports for consumer behavior tests, never production authorization.
mod coordinator_ports;
mod provider_access;
mod publication;
pub use coordinator_ports::{
    FixtureArgumentFingerprint, ObservedCoordinatorEvents, RefusedSessionConsent,
    ScriptedCommandSource, ScriptedCoordinatorIdentity, ScriptedCoordinatorProviders,
    UnavailableRunTools, fixture_outbox, fixture_outbox_with_work, fixture_publication_budget,
};
pub use provider_access::{RecordingProviderCredentialReceiver, ScriptedProviderCredentials};
pub use publication::{
    ScriptedPublicationTransport, ScriptedSnapshotEncoder, fixture_snapshot_budget,
};
mod coordinator_tools;
pub use coordinator_tools::ScriptedRunTools;
mod coordinator_engine;
pub use coordinator_engine::{ScriptedEngineFactory, ScriptedEngineTurn};
mod model_admission;
mod tool_ports;
pub use model_admission::ScriptedModelAdmission;
mod engine_events;
pub use engine_events::ScriptedEngineEvents;
pub use tool_ports::{
    ManualToolTimer, QueuedToolTasks, ScriptedBackgroundJournal, ScriptedSessionHandles,
    ScriptedSessionOpener, ScriptedToolAuthority,
};

mod login;
pub use login::ScriptedLoginAuthority;
mod summaries;
pub use summaries::RecordedSummaryDirectory;
