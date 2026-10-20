//! Codex account-log grammar, separate from its native reader.
use nocterm_ai::usage::{RateLimit, Window};

/// The plan limits in one line of a Codex session log, if it reports them.
pub(super) fn codex_limits(line: &str) -> Option<Vec<RateLimit>> {
    if !line.contains("\"rate_limits\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let limits = value.get("payload")?.get("rate_limits")?;
    let reached = limits
        .get("rate_limit_reached_type")
        .is_some_and(|reached| !reached.is_null());
    let parsed: Vec<_> = ["primary", "secondary"]
        .into_iter()
        .filter_map(|key| {
            let window = limits.get(key)?;
            let used = window.get("used_percent")?.as_f64()? / 100.;
            Some(RateLimit {
                window: Window::from_minutes(window.get("window_minutes")?.as_u64()?),
                used: Some(used.clamp(0., 1.)),
                resets_at: window.get("resets_at").and_then(serde_json::Value::as_u64),
                reached: reached && used >= 1.,
            })
        })
        .collect();
    (!parsed.is_empty()).then_some(parsed)
}
