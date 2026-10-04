//! What a chat uses: its context window, the tokens spent, and the plan
//! limits of the account behind the agent.
//!
//! ACP reports the context window (`usage_update`) and, at the end of a turn,
//! the session's token totals. Plan limits are not part of ACP: the Claude
//! adapter attaches them to `usage_update` under `_claude/rateLimit`, and
//! Codex writes them into its own session logs, which are read here.

use std::{
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

use crate::{acp, thread::Entry};

/// The span a plan limit counts usage over.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Window {
    FiveHour,
    Weekly,
    /// A weekly limit on one family of models, such as Opus.
    WeeklyModel(String),
    /// Another span, in minutes.
    Minutes(u64),
    /// Pay-as-you-go usage beyond the plan.
    Extra,
}

impl Window {
    pub fn label(&self) -> String {
        match self {
            Self::FiveHour => "5-hour limit".into(),
            Self::Weekly => "Weekly limit".into(),
            Self::WeeklyModel(model) => format!("Weekly limit · {model}"),
            Self::Minutes(minutes) if minutes % 1440 == 0 => {
                format!("{}-day limit", minutes / 1440)
            }
            Self::Minutes(minutes) if minutes % 60 == 0 => {
                format!("{}-hour limit", minutes / 60)
            }
            Self::Minutes(minutes) => format!("{minutes}-minute limit"),
            Self::Extra => "Extra usage".into(),
        }
    }

    fn from_minutes(minutes: u64) -> Self {
        match minutes {
            300 => Self::FiveHour,
            10_080 => Self::Weekly,
            minutes => Self::Minutes(minutes),
        }
    }
}

/// One plan limit.
#[derive(Clone, Debug, PartialEq)]
pub struct RateLimit {
    pub window: Window,
    /// The share used, from 0 to 1, when the agent tells.
    pub used: Option<f64>,
    /// When the window starts over, in seconds since the Unix epoch.
    pub resets_at: Option<u64>,
    /// Requests are refused until the window starts over.
    pub reached: bool,
}

/// The plan limits last reported for one agent, ordered by window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limits(Vec<RateLimit>);

impl Limits {
    pub fn iter(&self) -> impl Iterator<Item = &RateLimit> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Records `limit`, replacing the earlier report for its window.
    pub fn merge(&mut self, limit: RateLimit) {
        match self.0.iter_mut().find(|known| known.window == limit.window) {
            Some(known) => *known = limit,
            None => {
                self.0.push(limit);
                self.0.sort_by(|a, b| a.window.cmp(&b.window));
            }
        }
    }

    /// Replaces every limit: the source reports all of them at once.
    pub fn replace(&mut self, limits: Vec<RateLimit>) {
        self.0.clear();
        for limit in limits {
            self.merge(limit);
        }
    }
}

/// The plan limit the Claude adapter attached to `update`, if any.
pub fn claude_limit(update: &acp::UsageUpdate) -> Option<RateLimit> {
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

/// The plan limits in one line of a Codex session log, if it reports them.
pub fn codex_limits(line: &str) -> Option<Vec<RateLimit>> {
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

/// Whether `launch` runs Codex, whose limits are read from its logs.
pub fn is_codex(launch: &crate::AgentLaunch) -> bool {
    std::iter::once(&launch.command)
        .chain(&launch.args)
        .any(|part| part.contains("codex"))
}

/// Where Codex keeps its data.
pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
}

/// How much of the end of a session log is searched for limits.
const CODEX_TAIL_BYTES: u64 = 256 * 1024;

/// The plan limits Codex last wrote into its newest session log under
/// `home`. Logs are laid out by date, `sessions/YYYY/MM/DD/rollout-*.jsonl`.
pub fn read_codex_limits(home: &Path) -> Option<Vec<RateLimit>> {
    let mut dir = home.join("sessions");
    // Year, month, day: the greatest name is the newest.
    for _ in 0..3 {
        dir = newest(&dir, |path| path.is_dir())?;
    }
    let log = newest(&dir, |path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    })?;
    let mut file = std::fs::File::open(log).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(CODEX_TAIL_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    file.take(CODEX_TAIL_BYTES).read_to_end(&mut tail).ok()?;
    String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .find_map(codex_limits)
}

/// The entry of `dir` with the greatest name that passes `keep`.
fn newest(dir: &Path, keep: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| keep(path))
        .max_by(|a, b| a.file_name().cmp(&b.file_name()))
}

/// What fills the context window, estimated from the transcript.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Breakdown {
    /// The agent's own instructions and tool definitions, and whatever the
    /// transcript does not show: the rest of what the agent reported.
    pub agent: u64,
    pub messages: u64,
    pub reasoning: u64,
    pub tools: u64,
}

/// Roughly how many tokens a text takes.
fn tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4)
}

impl Breakdown {
    /// Splits the `used` tokens the agent reported by what the transcript
    /// shows. Agents that compact their context hold less than the
    /// transcript, so the transcript's parts are scaled down to fit.
    pub fn estimate(entries: &[Entry], used: u64) -> Self {
        let mut parts = Self::default();
        for entry in entries {
            match entry {
                Entry::User(blocks) => {
                    for block in blocks {
                        if let acp::ContentBlock::Text(text) = block {
                            parts.messages += tokens(&text.text);
                        }
                    }
                }
                Entry::Agent(text) => parts.messages += tokens(text),
                Entry::Thought(text) => parts.reasoning += tokens(text),
                Entry::Tool(call) => {
                    parts.tools += tokens(&call.title)
                        + call
                            .raw_input
                            .iter()
                            .chain(call.raw_output.iter())
                            .map(|value| tokens(&value.to_string()))
                            .sum::<u64>();
                }
                Entry::Content(_) => {}
            }
        }
        let shown = parts.messages + parts.reasoning + parts.tools;
        if shown > used {
            let scale = |part: u64| (part as u128 * used as u128 / shown.max(1) as u128) as u64;
            parts.messages = scale(parts.messages);
            parts.reasoning = scale(parts.reasoning);
            parts.tools = scale(parts.tools);
        }
        parts.agent = used - (parts.messages + parts.reasoning + parts.tools).min(used);
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(meta: serde_json::Value) -> acp::UsageUpdate {
        acp::UsageUpdate::new(10, 100).meta(meta.as_object().cloned())
    }

    #[test]
    fn claude_limits_come_from_usage_metadata() {
        let limit = claude_limit(&update(serde_json::json!({
            "_claude/rateLimit": {
                "status": "allowed_warning",
                "rateLimitType": "five_hour",
                "utilization": 0.82,
                "resetsAt": 1_791_588_105u64,
            }
        })))
        .unwrap();
        assert_eq!(limit.window, Window::FiveHour);
        assert_eq!(limit.used, Some(0.82));
        assert_eq!(limit.resets_at, Some(1_791_588_105));
        assert!(!limit.reached);

        let reached = claude_limit(&update(serde_json::json!({
            "_claude/rateLimit": {"status": "rejected", "rateLimitType": "seven_day_opus"}
        })))
        .unwrap();
        assert_eq!(reached.window, Window::WeeklyModel("Opus".into()));
        assert_eq!(reached.used, None);
        assert!(reached.reached);

        assert!(claude_limit(&acp::UsageUpdate::new(1, 2)).is_none());
        assert!(claude_limit(&update(serde_json::json!({"other": 1}))).is_none());
    }

    const CODEX_LINE: &str = r#"{"timestamp":"2026-10-03T17:42:09Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"codex","primary":{"used_percent":38.0,"window_minutes":10080,"resets_at":1791588105},"secondary":{"used_percent":7.5,"window_minutes":300,"resets_at":1791500000},"rate_limit_reached_type":null}}}"#;

    #[test]
    fn codex_limits_come_from_its_session_log() {
        let limits = codex_limits(CODEX_LINE).unwrap();
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].window, Window::Weekly);
        assert_eq!(limits[0].used, Some(0.38));
        assert_eq!(limits[0].resets_at, Some(1_791_588_105));
        assert_eq!(limits[1].window, Window::FiveHour);
        assert!(codex_limits(r#"{"payload":{"type":"agent_message"}}"#).is_none());
        assert!(codex_limits(r#"{"payload":{"rate_limits":{"primary":null}}}"#).is_none());
    }

    #[test]
    fn the_newest_codex_log_is_read() {
        let home = tempfile::tempdir().unwrap();
        let day = |path: &str| {
            let dir = home.path().join("sessions").join(path);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };
        std::fs::write(
            day("2026/09/30").join("rollout-a.jsonl"),
            CODEX_LINE.replace("38.0", "99.0"),
        )
        .unwrap();
        let newest = day("2026/10/03");
        std::fs::write(newest.join("rollout-a.jsonl"), "{}\n").unwrap();
        std::fs::write(
            newest.join("rollout-b.jsonl"),
            format!("{CODEX_LINE}\n{{\"payload\":{{\"type\":\"agent_message\"}}}}\n"),
        )
        .unwrap();

        let limits = read_codex_limits(home.path()).unwrap();

        assert_eq!(limits[0].used, Some(0.38));
        assert!(read_codex_limits(&home.path().join("missing")).is_none());
    }

    #[test]
    fn limits_merge_by_window_in_order() {
        let limit = |window, used| RateLimit {
            window,
            used: Some(used),
            resets_at: None,
            reached: false,
        };
        let mut limits = Limits::default();
        limits.merge(limit(Window::Weekly, 0.1));
        limits.merge(limit(Window::FiveHour, 0.2));
        limits.merge(limit(Window::Weekly, 0.3));
        assert_eq!(
            limits.iter().map(|limit| limit.used).collect::<Vec<_>>(),
            [Some(0.2), Some(0.3)]
        );
        assert_eq!(Window::Minutes(2880).label(), "2-day limit");
        assert_eq!(Window::Minutes(120).label(), "2-hour limit");
    }

    #[test]
    fn breakdown_splits_reported_usage_by_transcript() {
        let entries = vec![
            Entry::User(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "a".repeat(400),
            ))]),
            Entry::Thought("b".repeat(80)),
            Entry::Agent("c".repeat(400)),
        ];
        let parts = Breakdown::estimate(&entries, 1_000);
        assert_eq!(parts.messages, 200);
        assert_eq!(parts.reasoning, 20);
        assert_eq!(parts.agent, 780);

        // A compacted context holds less than the transcript.
        let parts = Breakdown::estimate(&entries, 110);
        assert!(parts.messages + parts.reasoning + parts.tools <= 110);
        assert_eq!(parts.messages, 100);
    }
}
