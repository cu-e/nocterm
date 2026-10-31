//! Conservative finite model spending; descriptive values are not execution permits.
mod ledger;
mod managed;
mod policy;
mod pricing;

pub use ledger::{
    InferenceCallSnapshot, InferenceCharge, InferenceLedger, InferenceReservation,
    InferenceSnapshot,
};
pub use managed::{MANAGED_PRICING, MANAGED_PROVIDER};
pub use policy::{InferenceGrant, InferenceTask, ModelSelection};
pub use pricing::{ConservativeModelPricing, ModelPricing, ModelQuote};

#[derive(Clone, Debug, thiserror::Error)]
pub enum InferenceError {
    #[error("the model, run or original inference grant binding differs")]
    Binding,
    #[error("the inference allowance must be finite and positive")]
    InvalidLimits,
    #[error("the model request exceeds its configured trusted pricing ceiling")]
    PricingLimit,
    #[error("the inference task or grant has expired")]
    Expired,
    #[error("the observed clock moved backwards; this run needs fresh authority")]
    ClockRollback,
    #[error("the inference task or grant budget is exhausted")]
    BudgetExceeded,
    #[error("the original model attempt or observation conflicts")]
    Conflict,
    #[error("the provider observation exceeds its original reservation")]
    InvalidObservation,
    #[error("the cumulative reported provider usage overflows its representation")]
    UsageOverflow,
    #[error("the restored inference budget is inconsistent")]
    InvalidSnapshot,
}
