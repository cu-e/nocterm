//! Which record an invalid projection input belonged to. Display-safe: names a kind of
//! record, never its content.
use std::fmt;

/// The record, or the stage of a record's journey, that was invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    /// The snapshot baseline's facts or anchor.
    SnapshotBaseline,
    /// An approval request or its resolution.
    Approval,
    /// The expiry request of an approval answer.
    ApprovalRequest,
    /// The timing of a prepared conversation command.
    Preparation,
    /// A run's identity or its status transitions.
    Run,
    /// A tool call or its result.
    Tool,
    /// The provider lease a device grants to a runner.
    ProviderAccess,
    /// An event that does not fit the order or state of the projection.
    Event,
    /// A prompt the runner accepted.
    Prompt,
    /// A coordinate, epoch or identity inside a signed observation.
    Coordinate,
    /// A signed observation's payload.
    Observation,
    /// Streamed text or a JSON value of a signed observation.
    Json,
    /// A managed-session payload that this client does not project.
    ManagedSession,
    /// A chat head or head anchor of the hub's catalog.
    Head,
    /// The encrypted projection cache.
    Cache,
    /// The encrypted client storage.
    Storage,
    /// The encrypted client outbox.
    Outbox,
    /// A page of verified records being replayed.
    Replay,
    /// A command being prepared, staged or delivered.
    Command,
    /// The conversation's prompt policy.
    Policy,
    /// A command waiting in the chat's pending queue.
    PendingCommand,
    /// The limits and model a chat's setup configured.
    ChatSetup,
    /// The Cloud relay protocol.
    Protocol,
    /// The capability proof of a Cloud request.
    Capability,
    /// The relay's wire format.
    RelayFormat,
    /// The Cloud command wire format.
    CommandFormat,
    /// The chat's key material.
    KeyMaterial,
    /// Command encryption.
    Encryption,
    /// The runner's identity proof.
    RunnerProof,
    /// A command's semantic payload.
    Payload,
    /// A head probe or its witness.
    HeadProbe,
    /// Head encryption or authentication.
    HeadCrypto,
    /// A runner observation that failed authentication or canonical decoding.
    Authentication,
    /// Test data.
    Fixture,
}

impl RecordKind {
    fn name(self) -> &'static str {
        match self {
            Self::SnapshotBaseline => "snapshot baseline",
            Self::Approval => "approval",
            Self::ApprovalRequest => "approval request",
            Self::Preparation => "command timing",
            Self::Run => "run",
            Self::Tool => "tool call",
            Self::ProviderAccess => "provider lease",
            Self::Event => "event order",
            Self::Prompt => "prompt",
            Self::Coordinate => "coordinate",
            Self::Observation => "observation",
            Self::Json => "text or JSON value",
            Self::ManagedSession => "managed-session payload",
            Self::Head => "chat head",
            Self::Cache => "projection cache",
            Self::Storage => "client storage",
            Self::Outbox => "client outbox",
            Self::Replay => "replayed page",
            Self::Command => "command",
            Self::Policy => "prompt policy",
            Self::PendingCommand => "pending command",
            Self::ChatSetup => "chat setup",
            Self::Protocol => "relay protocol",
            Self::Capability => "capability proof",
            Self::RelayFormat => "relay format",
            Self::CommandFormat => "command format",
            Self::KeyMaterial => "key material",
            Self::Encryption => "command encryption",
            Self::RunnerProof => "runner proof",
            Self::Payload => "command payload",
            Self::HeadProbe => "head probe",
            Self::HeadCrypto => "head authentication",
            Self::Authentication => "runner observation",
            Self::Fixture => "test fixture",
        }
    }
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
