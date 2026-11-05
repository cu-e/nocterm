use super::{InferenceError, ModelSelection};
use crate::{
    engine::{MAX_MODEL_OUTPUT_TOKENS, ModelCallRequest},
    payload::MAX_LOGICAL_PAYLOAD_BYTES,
};

/// Trusted configured upper bound, never an invoice or a token estimate from bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelQuote {
    selection: ModelSelection,
    spend: u64,
}
impl ModelQuote {
    pub fn new(selection: ModelSelection, spend: u64) -> Result<Self, InferenceError> {
        if spend == 0 {
            return Err(InferenceError::InvalidLimits);
        }
        Ok(Self { selection, spend })
    }
    pub fn selection(&self) -> &ModelSelection {
        &self.selection
    }
    pub fn spend_micros(&self) -> u64 {
        self.spend
    }
}

/// Required trusted collaborator bound to configured provider/model/policy. A
/// quote must cover all billable components within its accepted request/output
/// caps. No prompt-derived pricing, network lookup or bytes-to-tokens guess.
pub trait ModelPricing: Send + Sync {
    fn selection(&self) -> &ModelSelection;
    fn quote(&self, request: &ModelCallRequest) -> Result<ModelQuote, InferenceError>;
}

/// Finite administrator-configured worst-case cost per accepted call. A provider
/// with unbounded/unknown pricing cannot be represented by this implementation.
/// This conservative charge is retained when the provider reports no cost.
pub struct ConservativeModelPricing {
    selection: ModelSelection,
    request_bytes: usize,
    output_tokens: u64,
    spend: u64,
}
impl ConservativeModelPricing {
    pub fn new(
        selection: ModelSelection,
        request_bytes: usize,
        output_tokens: u64,
        spend: u64,
    ) -> Result<Self, InferenceError> {
        if request_bytes == 0
            || request_bytes > MAX_LOGICAL_PAYLOAD_BYTES
            || output_tokens == 0
            || output_tokens > MAX_MODEL_OUTPUT_TOKENS
            || spend == 0
        {
            return Err(InferenceError::InvalidLimits);
        }
        Ok(Self {
            selection,
            request_bytes,
            output_tokens,
            spend,
        })
    }
}
impl ModelPricing for ConservativeModelPricing {
    fn selection(&self) -> &ModelSelection {
        &self.selection
    }
    fn quote(&self, request: &ModelCallRequest) -> Result<ModelQuote, InferenceError> {
        if request.request_bytes() > self.request_bytes {
            return Err(InferenceError::RequestTooLarge {
                limit: self.request_bytes,
            });
        }
        if request.output_token_limit() > self.output_tokens {
            return Err(InferenceError::PricingLimit);
        }
        ModelQuote::new(self.selection.clone(), self.spend)
    }
}
