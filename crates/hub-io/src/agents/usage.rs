//! The person's own usage per model, from the hub's usage report.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nocterm_cloud_agents_model::{CloudAgentError, ModelUsage, UsageSample};
use nocterm_hub_model::agents::AgentScope;
use nocterm_hub_proto::v1;

use super::error::agent_error;
use crate::devices::Captured;

const SECONDS_PER_DAY: u64 = 86_400;

/// Days since the Unix epoch (UTC) of `now`.
fn day_of(now: SystemTime) -> u32 {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    u32::try_from(seconds / SECONDS_PER_DAY).unwrap_or(u32::MAX)
}

/// The inclusive day range of the last `days` days ending `now`.
fn window(now: SystemTime, days: u32) -> (u32, u32) {
    let today = day_of(now);
    (today.saturating_sub(days.saturating_sub(1)), today)
}

pub(super) async fn personal(
    captured: Arc<Captured>,
    days: u32,
) -> Result<Vec<ModelUsage>, CloudAgentError> {
    let address = captured.hub.address().clone();
    let (from_day, to_day) = window(SystemTime::now(), days);
    let report = captured
        .client
        .agents()
        .usage(v1::GetUsageRequest {
            scope: Some(AgentScope::Account.into()),
            agent_id: None,
            account_id: None,
            from_day,
            to_day,
        })
        .await
        .map_err(|error| agent_error(&address, error))?;
    Ok(ModelUsage::by_model(report.rows.into_iter().map(|row| {
        UsageSample {
            model: row.model,
            runs: row.runs,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            spend_micros: row.spend_micros,
        }
    })))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_window_ends_today_and_spans_the_requested_days() {
        let now = UNIX_EPOCH + Duration::from_secs(100 * SECONDS_PER_DAY + 5);
        assert_eq!(window(now, 30), (71, 100));
        assert_eq!(window(now, 1), (100, 100));
        assert_eq!(
            window(UNIX_EPOCH, 30),
            (0, 0),
            "the window never starts before day 0"
        );
    }
}
