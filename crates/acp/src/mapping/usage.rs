//! Provider-specific rate-limit metadata is interpreted only at the live adapter.
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::{
    model::usage::{ContextUsage, Cost, TokenTotals},
    usage::{RateLimit, Window},
};
pub(super) fn tokens(value: wire::Usage) -> TokenTotals {
    TokenTotals {
        total: value.total_tokens,
        input: value.input_tokens,
        output: value.output_tokens,
        thought: value.thought_tokens,
        cache_read: value.cached_read_tokens,
        cache_write: value.cached_write_tokens,
    }
}
pub(super) fn context(value: &wire::UsageUpdate) -> ContextUsage {
    ContextUsage {
        used: value.used,
        size: value.size,
        cost: value.cost.as_ref().map(|cost| Cost {
            amount: cost.amount,
            currency: cost.currency.clone(),
        }),
    }
}
pub(super) fn limits(value: &wire::UsageUpdate) -> Vec<RateLimit> {
    claude_limit(value).into_iter().collect()
}
fn claude_limit(update: &wire::UsageUpdate) -> Option<RateLimit> {
    let info = update.meta.as_ref()?.get("_claude/rateLimit")?;
    let window = match info.get("rateLimitType")?.as_str()? {
        "five_hour" => Window::FiveHour,
        "seven_day" => Window::Weekly,
        "seven_day_opus" => Window::WeeklyModel("Opus".into()),
        "seven_day_sonnet" => Window::WeeklyModel("Sonnet".into()),
        "overage" | "seven_day_overage_included" => Window::Extra,
        _ => return None,
    };
    Some(RateLimit {
        window,
        used: info
            .get("utilization")
            .and_then(serde_json::Value::as_f64)
            .map(|used| used.clamp(0., 1.)),
        resets_at: info.get("resetsAt").and_then(serde_json::Value::as_u64),
        reached: info.get("status").and_then(serde_json::Value::as_str) == Some("rejected"),
    })
}
