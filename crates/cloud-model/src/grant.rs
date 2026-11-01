//! Finite execution admission over exact original coordinates and supplied time.
use crate::{
    authority::{AuthorityError, CommandIssuer, ExecutionAuthority},
    identity::{ChatControlRevision, CommandId, Timestamp},
};
use nocterm_ai::{TerminalCall, TerminalCallError, ToolCapability};

pub const MAX_TASK_GRANT_MILLIS: u64 = 30 * 60 * 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities(u8);
impl Capabilities {
    pub const NONE: Self = Self(0);
    pub const READ_ONLY: Self = Self(1);
    pub const ALL: Self = Self(15);
    pub fn with(self, capability: ToolCapability) -> Self {
        Self(self.0 | bit(capability))
    }
    pub fn bits(self) -> u8 {
        self.0
    }
    pub fn from_bits(bits: u8) -> Result<Self, GrantError> {
        if bits & !15 != 0 {
            return Err(GrantError::InvalidCapabilities);
        }
        Ok(Self(bits))
    }
    pub fn permits(self, capability: ToolCapability) -> bool {
        self.0 & bit(capability) != 0
    }
}
fn bit(capability: ToolCapability) -> u8 {
    match capability {
        ToolCapability::Read => 1,
        ToolCapability::LiveInput => 2,
        ToolCapability::Execution => 4,
        ToolCapability::Connection => 8,
        // Transfers share the execution bit: the grant byte format is a contract.
        ToolCapability::Transfer => 4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrantWindow {
    issued: Timestamp,
    expires: Timestamp,
}
impl GrantWindow {
    pub fn new(issued: Timestamp, expires: Timestamp) -> Result<Self, GrantError> {
        let duration = expires
            .elapsed_since(issued)
            .ok_or(GrantError::InvalidDeadline)?;
        if duration == 0 || duration > MAX_TASK_GRANT_MILLIS {
            return Err(GrantError::InvalidDeadline);
        }
        Ok(Self { issued, expires })
    }
    pub fn issued(&self) -> Timestamp {
        self.issued
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantRevocation {
    Explicit,
    ClockRollback,
}

/// Original durable grant admission; token refresh never renames its issuer.
/// Revocation selects explicit grant ids or this original control cutoff, not a
/// newly signed-in device's current token digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantOrigin {
    issuer: CommandIssuer,
    command: CommandId,
    control: ChatControlRevision,
}
impl GrantOrigin {
    pub fn new(issuer: CommandIssuer, command: CommandId, control: ChatControlRevision) -> Self {
        Self {
            issuer,
            command,
            control,
        }
    }
    pub fn issuer(&self) -> &CommandIssuer {
        &self.issuer
    }
    pub fn command(&self) -> CommandId {
        self.command
    }
    pub fn control(&self) -> ChatControlRevision {
        self.control
    }
}

/// Immutable finite policy. Construction does not verify a signature or renew
/// an already admitted grant; the host's durable epoch owner decides admission.
#[derive(Clone, Debug)]
pub struct GrantPolicy {
    authority: ExecutionAuthority,
    origin: GrantOrigin,
    capabilities: Capabilities,
    window: GrantWindow,
    operations: u32,
}
impl GrantPolicy {
    pub fn new(
        authority: ExecutionAuthority,
        origin: GrantOrigin,
        capabilities: Capabilities,
        window: GrantWindow,
        operations: u32,
    ) -> Result<Self, GrantError> {
        if operations == 0 {
            return Err(GrantError::InvalidBudget);
        }
        Ok(Self {
            authority,
            origin,
            capabilities,
            window,
            operations,
        })
    }
    pub fn authority(&self) -> &ExecutionAuthority {
        &self.authority
    }
    pub fn origin(&self) -> &GrantOrigin {
        &self.origin
    }
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
    pub fn window(&self) -> GrantWindow {
        self.window
    }
    pub fn operations(&self) -> u32 {
        self.operations
    }
}

#[derive(Clone, Debug)]
pub struct GrantSnapshot {
    policy: GrantPolicy,
    remaining: u32,
    observed: Timestamp,
    revoked: Option<GrantRevocation>,
}
impl GrantSnapshot {
    pub fn new(
        policy: GrantPolicy,
        remaining: u32,
        observed: Timestamp,
        revoked: Option<GrantRevocation>,
    ) -> Result<Self, GrantError> {
        if remaining > policy.operations || observed < policy.window.issued {
            return Err(GrantError::InvalidRecovery);
        }
        Ok(Self {
            policy,
            remaining,
            observed,
            revoked,
        })
    }
    pub fn policy(&self) -> &GrantPolicy {
        &self.policy
    }
    pub fn remaining(&self) -> u32 {
        self.remaining
    }
    pub fn observed(&self) -> Timestamp {
        self.observed
    }
    pub fn revocation(&self) -> Option<GrantRevocation> {
        self.revoked
    }
}

/// Admission policy only. Signature verification and session lookup precede its use.
/// Recreating this owner is not renewal: the journal must fence previous grant epochs.
#[derive(Clone, Debug)]
pub struct GrantState {
    snapshot: GrantSnapshot,
}
impl GrantState {
    pub fn new(
        authority: ExecutionAuthority,
        origin: GrantOrigin,
        capabilities: Capabilities,
        window: GrantWindow,
        operations: u32,
    ) -> Result<Self, GrantError> {
        let policy = GrantPolicy::new(authority, origin, capabilities, window, operations)?;
        Ok(Self {
            snapshot: GrantSnapshot {
                policy,
                remaining: operations,
                observed: window.issued,
                revoked: None,
            },
        })
    }
    pub fn from_snapshot(snapshot: GrantSnapshot) -> Self {
        Self { snapshot }
    }
    pub fn snapshot(&self) -> GrantSnapshot {
        self.snapshot.clone()
    }
    pub fn policy(&self) -> &GrantPolicy {
        &self.snapshot.policy
    }
    pub fn authority(&self) -> &ExecutionAuthority {
        self.policy().authority()
    }
    pub fn origin(&self) -> &GrantOrigin {
        self.policy().origin()
    }
    pub fn remaining_operations(&self) -> u32 {
        self.snapshot.remaining
    }
    pub fn revocation(&self) -> Option<GrantRevocation> {
        self.snapshot.revoked
    }
    pub fn expires(&self) -> Timestamp {
        self.policy().window.expires
    }
    pub fn revoke(&mut self) {
        self.snapshot
            .revoked
            .get_or_insert(GrantRevocation::Explicit);
    }

    /// Validates arguments and consumes one finite operation reservation only on success.
    /// A returned decision is not a durable receipt or a reusable I/O capability.
    pub fn authorize(
        &mut self,
        expected: &ExecutionAuthority,
        call: &TerminalCall,
        now: Timestamp,
    ) -> Result<(), GrantError> {
        self.revalidate_reserved(expected, call, now)?;
        if self.snapshot.remaining == 0 {
            return Err(GrantError::BudgetExceeded);
        }
        self.snapshot.remaining -= 1;
        Ok(())
    }

    /// Rechecks a previously reserved original effect without spending another
    /// operation. The host must first match its actual durable dispatch permit;
    /// this method alone neither proves a reservation nor authorizes a new call.
    pub fn revalidate_reserved(
        &mut self,
        expected: &ExecutionAuthority,
        call: &TerminalCall,
        now: Timestamp,
    ) -> Result<(), GrantError> {
        self.policy().authority.require_same(expected)?;
        if now < self.snapshot.observed {
            self.snapshot.revoked = Some(GrantRevocation::ClockRollback);
            return Err(GrantError::ClockRollback);
        }
        self.snapshot.observed = now;
        if self.snapshot.revoked.is_some() {
            return Err(GrantError::Revoked);
        }
        if now >= self.policy().window.expires {
            return Err(GrantError::Expired);
        }
        if !self.policy().capabilities.permits(call.capability()) {
            return Err(GrantError::CapabilityDenied);
        }
        call.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum GrantError {
    #[error("the grant contains an unknown tool capability")]
    InvalidCapabilities,
    #[error("the restored grant counters or time are invalid")]
    InvalidRecovery,
    #[error("the original execution authority does not match")]
    Authority(#[from] AuthorityError),
    #[error("the grant deadline must be finite and within thirty minutes")]
    InvalidDeadline,
    #[error("the grant must reserve at least one operation")]
    InvalidBudget,
    #[error("the supplied clock moved backwards; fresh authority is required")]
    ClockRollback,
    #[error("the execution grant is revoked")]
    Revoked,
    #[error("the execution grant has expired")]
    Expired,
    #[error("the grant does not permit this tool capability")]
    CapabilityDenied,
    #[error("the grant operation budget is exhausted")]
    BudgetExceeded,
    #[error("the tool arguments are invalid")]
    InvalidArguments(#[from] TerminalCallError),
}
