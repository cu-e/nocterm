//! Native usage facts retain every allocated currency/window text until adoption.
use super::NativeOwned;
use crate::model::{
    json::deferred::JsonDecodeError,
    usage::{ContextUsage, Cost},
};
pub fn cost(amount: f64, currency: NativeOwned<String>) -> NativeOwned<Cost> {
    let (currency, allocation) = currency.into_parts();
    NativeOwned::from_parts(Cost { amount, currency }, allocation)
}
pub fn context(used: u64, size: u64, cost: NativeOwned<Option<Cost>>) -> NativeOwned<ContextUsage> {
    let (cost, allocation) = cost.into_parts();
    NativeOwned::from_parts(ContextUsage { used, size, cost }, allocation)
}
pub fn window(
    minutes: u64,
    ledger: &crate::model::json::deferred::DecodeLedger,
) -> Result<NativeOwned<crate::usage::Window>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        crate::usage::Window::from_minutes(minutes),
        ledger.reserve(0)?,
    ))
}
pub fn weekly_model(model: NativeOwned<String>) -> NativeOwned<crate::usage::Window> {
    let (model, allocation) = model.into_parts();
    NativeOwned::from_parts(crate::usage::Window::WeeklyModel(model), allocation)
}
pub fn extra(
    ledger: &crate::model::json::deferred::DecodeLedger,
) -> Result<NativeOwned<crate::usage::Window>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        crate::usage::Window::Extra,
        ledger.reserve(0)?,
    ))
}
pub fn rate_limit(
    window: NativeOwned<crate::usage::Window>,
    used: Option<f64>,
    resets_at: Option<u64>,
    reached: bool,
) -> NativeOwned<crate::usage::RateLimit> {
    let (window, allocation) = window.into_parts();
    NativeOwned::from_parts(
        crate::usage::RateLimit {
            window,
            used,
            resets_at,
            reached,
        },
        allocation,
    )
}
