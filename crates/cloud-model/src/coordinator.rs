//! Host command/control contracts; plain values do not verify signatures or fsync.
mod command;
mod delivery;
mod error;
mod event;
mod observations;
pub use observations::{
    BackgroundObservation, ObservedBackground, ObservedEffect, ObservedJobWindow, ObservedPrompt,
};
pub(crate) mod intake;
mod ports;
mod provider_access;
mod session;
mod supervision;
pub use command::{
    ApprovalAnswer, ApprovalChoice, CommandAcknowledgement, CommandBody, GrantClass,
    GrantReference, IssuerRevocation, IssuerSubject, PromptAllowance, PromptRequest, ProviderOffer,
    ReceivedCommand, RunDestination, SessionConsent,
};
pub use error::CoordinatorError;
pub use event::{CoordinatorEvent, RunStatus, ToolObservation};
pub use ports::{
    AuthenticatedCommandSource, ChatRetirement, ChatRetirementRequest, ConfiguredModel, CoordinatorEventEncoder, CoordinatorIdentity,
    CoordinatorJournal, CoordinatorProviders, SessionConsentConsumer,
};
pub use provider_access::{ProviderCredentialReceiver, ProviderCredentialSource};
pub use session::{
    PreparedRunSession, PublishedRunSession, RunSessionBroker, SessionAttempt,
    SessionAttemptCancellation, SessionFactSink, SessionFailure, SessionObservation,
    SessionOutcome, SessionRequest, SessionStartup,
};
pub use supervision::{DrainReport, SupervisorLimits, SupervisorPhase, TaskSnapshot};

pub use intake::{
    CommandReceiveBudget, IntakeError, IntakeLane, IntakeLimits, IntakePoolLimits, IntakeUsage,
};

pub use delivery::{CommandInput, CommandRefusal, CommandReply, ResidentCommand, RetryReason};
