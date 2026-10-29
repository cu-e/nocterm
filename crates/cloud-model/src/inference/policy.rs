use super::InferenceError;
use crate::{
    authority::RunBinding,
    engine::ModelBudget,
    grant::{GrantOrigin, GrantWindow},
    identity::{GrantEpoch, GrantId, NamespaceId},
};

/// Configured provider/model/pricing identity; no endpoint or credential may be
/// selected through model text. The injected factory and pricing must match it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    provider: NamespaceId,
    model: NamespaceId,
    pricing: NamespaceId,
}
impl ModelSelection {
    pub fn new(provider: NamespaceId, model: NamespaceId, pricing: NamespaceId) -> Self {
        Self {
            provider,
            model,
            pricing,
        }
    }
    pub fn provider(&self) -> &NamespaceId {
        &self.provider
    }
    pub fn model(&self) -> &NamespaceId {
        &self.model
    }
    pub fn pricing(&self) -> &NamespaceId {
        &self.pricing
    }
}

/// Immutable cumulative task ceiling, independent from renewable grant IDs.
#[derive(Clone, Debug)]
pub struct InferenceTask {
    run: RunBinding,
    selection: ModelSelection,
    budget: ModelBudget,
    spend: u64,
    window: GrantWindow,
}
impl InferenceTask {
    pub fn new(
        run: RunBinding,
        selection: ModelSelection,
        budget: ModelBudget,
        spend: u64,
        window: GrantWindow,
    ) -> Result<Self, InferenceError> {
        if spend == 0 {
            return Err(InferenceError::InvalidLimits);
        }
        Ok(Self {
            run,
            selection,
            budget,
            spend,
            window,
        })
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn selection(&self) -> &ModelSelection {
        &self.selection
    }
    pub fn budget(&self) -> ModelBudget {
        self.budget
    }
    pub fn spend_micros(&self) -> u64 {
        self.spend
    }
    pub fn window(&self) -> GrantWindow {
        self.window
    }
    /// Checks only descriptive ceiling containment; signatures, current epochs
    /// and durable reservation remain separate host checks.
    pub fn contains(&self, other: &Self) -> bool {
        self.run == other.run
            && self.selection == other.selection
            && other.budget.calls() <= self.budget.calls()
            && other.budget.turns() <= self.budget.turns()
            && other.budget.input_bytes() <= self.budget.input_bytes()
            && other.budget.output_bytes() <= self.budget.output_bytes()
            && other.budget.output_tokens() <= self.budget.output_tokens()
            && other.spend <= self.spend
            && other.window.issued() >= self.window.issued()
            && other.window.expires() <= self.window.expires()
    }
}

/// Original durable grant attribution. A new grant never resets task charges.
/// Construction validates descriptive bounds, not signature, current issuer,
/// local grant epoch or a durable reservation. Those remain host responsibilities.
#[derive(Clone, Debug)]
pub struct InferenceGrant {
    id: GrantId,
    epoch: GrantEpoch,
    origin: GrantOrigin,
    allowance: InferenceTask,
}
impl InferenceGrant {
    pub fn new(
        id: GrantId,
        epoch: GrantEpoch,
        origin: GrantOrigin,
        allowance: InferenceTask,
    ) -> Self {
        Self {
            id,
            epoch,
            origin,
            allowance,
        }
    }
    pub fn id(&self) -> GrantId {
        self.id
    }
    pub fn epoch(&self) -> GrantEpoch {
        self.epoch
    }
    pub fn origin(&self) -> &GrantOrigin {
        &self.origin
    }
    pub fn allowance(&self) -> &InferenceTask {
        &self.allowance
    }
}
