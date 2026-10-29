//! The host's durable barrier before one actual provider HTTP request.
use super::{EngineError, UsageObservation};
use crate::{authority::RunBinding, identity::StepId, payload::MAX_LOGICAL_PAYLOAD_BYTES};
use futures::future::BoxFuture;

pub const MAX_MODEL_OUTPUT_TOKENS: u64 = 1_000_000;

/// Descriptive request coordinates and bounds. Construction validates shape;
/// it neither verifies a command nor commits a budget reservation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCallRequest {
    run: RunBinding,
    step: StepId,
    request_bytes: usize,
    output_token_limit: u64,
}
impl ModelCallRequest {
    pub fn new(
        run: RunBinding,
        step: StepId,
        request_bytes: usize,
        output_token_limit: u64,
    ) -> Result<Self, EngineError> {
        if request_bytes == 0
            || request_bytes > MAX_LOGICAL_PAYLOAD_BYTES
            || output_token_limit == 0
            || output_token_limit > MAX_MODEL_OUTPUT_TOKENS
        {
            return Err(EngineError::BudgetExceeded);
        }
        Ok(Self {
            run,
            step,
            request_bytes,
            output_token_limit,
        })
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn step(&self) -> StepId {
        self.step
    }
    pub fn request_bytes(&self) -> usize {
        self.request_bytes
    }
    pub fn output_token_limit(&self) -> u64 {
        self.output_token_limit
    }
}

/// Installed by the host for the original run/step/attempt, writer and grant.
/// The engine receives no verified-command producer or authority constructor.
pub trait ModelCallAdmission: Send + Sync {
    /// Success follows the durable BudgetReserved transaction. The byte count
    /// is the actual provider wire body, not an estimated prompt/history size.
    /// The same transaction publishes the cumulative optional usage snapshot,
    /// including unknown totals for this newly unobserved call. Unknown usage
    /// retains the reservation and never implies a refund.
    fn reserve(&self, request: ModelCallRequest) -> BoxFuture<'static, Result<(), EngineError>>;

    /// Immutable verified observation for this original attempt. Success follows
    /// durable persistence before Usage progress and Tools/Complete; changed duplicates conflict.
    /// The original ledger and complete cumulative usage fact commit together;
    /// publication does not depend on a later SDK Usage callback. This is not a
    /// provider invoice or authority to refund a failed advance.
    fn reconcile(
        &self,
        request: ModelCallRequest,
        observation: ModelCallObservation,
    ) -> BoxFuture<'static, Result<(), EngineError>>;

    /// Repeatable, non-consuming check of original source/control/writer and
    /// deadlines immediately before I/O, including after reserve's await.
    fn check_current(&self, request: &ModelCallRequest) -> Result<(), EngineError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCallObservation {
    usage: UsageObservation,
    output_bytes: usize,
}
impl ModelCallObservation {
    pub fn new(usage: UsageObservation, output_bytes: usize) -> Result<Self, EngineError> {
        if output_bytes > MAX_LOGICAL_PAYLOAD_BYTES {
            return Err(EngineError::BudgetExceeded);
        }
        Ok(Self {
            usage,
            output_bytes,
        })
    }
    pub fn usage(&self) -> UsageObservation {
        self.usage
    }
    pub fn output_bytes(&self) -> usize {
        self.output_bytes
    }
}
