//! What a chat uses: its context window, the tokens spent, and the plan
//! limits of the account behind the agent.
//!
//! Adapters report context/token totals and semantic plan limits. Native account
//! logs are read through an injected source; provider metadata is not interpreted here.

mod source;
pub use source::{AccountUsageSource, UsageAgent, UsageReadError};

use crate::{model::content::Content, thread::Entry};

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

    pub fn from_minutes(minutes: u64) -> Self {
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
                        if let Content::Text(text) = block {
                            parts.messages += tokens(&text.text);
                        }
                    }
                }
                Entry::Agent(text) => parts.messages += tokens(text),
                Entry::Thought(text) => parts.reasoning += tokens(text),
                Entry::Tool(call) => {
                    parts.tools += tokens(&call.title)
                        + call
                            .arguments
                            .iter()
                            .chain(call.result.iter())
                            .map(|value| tokens(&value.to_string()))
                            .sum::<u64>();
                }
                Entry::Content(_) => {}
            }
        }
        parts.scale_to(used)
    }

    /// Borrow the real immutable physical rows. JSON uses the same compact
    /// bytes as the original display estimate without allocating those bytes.
    pub fn estimate_managed(
        entries: &[crate::history::physical::RowSlot],
        used: u64,
    ) -> Result<Self, crate::history::MeasurementError> {
        use crate::history::physical::{ContentSlot, RowBody};
        let mut parts = Self::default();
        for entry in entries {
            match entry.view() {
                RowBody::User(values) => {
                    for value in values.contents() {
                        if let ContentSlot::Text(value) = value {
                            parts.messages += tokens(&value.text());
                        }
                    }
                }
                RowBody::Agent(value) => parts.messages += tokens(&value.read()),
                RowBody::Thought(value) => parts.reasoning += tokens(&value.read()),
                RowBody::Tool(value) => {
                    parts.tools += tokens(&value.title());
                    for json in value.arguments().into_iter().chain(value.result()) {
                        let bytes = crate::history::measure_json(&*json)?.encoded_bytes;
                        parts.tools += (bytes as u64).div_ceil(4);
                    }
                }
                RowBody::Content(_) => {}
            }
        }
        Ok(parts.scale_to(used))
    }
    fn scale_to(mut self, used: u64) -> Self {
        let parts = &mut self;
        let shown = parts.messages + parts.reasoning + parts.tools;
        if shown > used {
            let scale = |part: u64| (part as u128 * used as u128 / shown.max(1) as u128) as u64;
            parts.messages = scale(parts.messages);
            parts.reasoning = scale(parts.reasoning);
            parts.tools = scale(parts.tools);
        }
        parts.agent = used - (parts.messages + parts.reasoning + parts.tools).min(used);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Entry::User(vec![Content::Text(crate::model::content::Text {
                text: "a".repeat(400),
                annotations: None,
                extensions: None,
            })]),
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

#[cfg(test)]
mod managed_tests {
    use super::*;
    use crate::{
        history::physical::SnapshotBuilder,
        model::{
            json::deferred::{DecodeLedger, literal::LiteralBuilder},
            native::{NativeList, NativeOwned},
            retention::{
                RetentionAuthority,
                document::{
                    DocumentRetention,
                    tests::{snapshot, text},
                },
            },
            tools::{ToolKind, ToolStatus},
        },
    };
    #[test]
    fn managed_estimate_matches_original_text_json_and_compaction_rules() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
        let mut foreground = roles.take_foreground().unwrap();
        let mut live = roles.take_live().unwrap();
        live.replace(snapshot(&foreground, "")).unwrap();
        let ledger =
            DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
        let mut content = NativeList::new(&ledger, 1).unwrap();
        content
            .push(crate::model::native::content::from_text(
                crate::model::native::content::text_content(
                    text(&ledger, &"message Ω".repeat(43)),
                    NativeOwned::absent(&ledger).unwrap(),
                    NativeOwned::absent(&ledger).unwrap(),
                )
                .unwrap(),
            ))
            .unwrap();
        let mut json = LiteralBuilder::new(&ledger).unwrap();
        let mut encoded = json.exact_text("quote\"\\\nΩ".len()).unwrap();
        encoded.append("quote\"\\\nΩ").unwrap();
        json.string(encoded.finish().unwrap()).unwrap();
        let arguments =
            crate::model::native::content::json(json.finish_native().unwrap(), &ledger).unwrap();
        let tool = crate::history::native::entries::tool_entry(
            crate::model::native::tools::record(
                crate::model::native::tools::header(
                    crate::model::native::tools::tool_id(text(&ledger, "tool")).unwrap(),
                    text(&ledger, "title Ω"),
                    NativeOwned::absent(&ledger).unwrap(),
                    ToolKind::Read,
                    ToolStatus::Pending,
                )
                .unwrap(),
                NativeList::new(&ledger, 0).unwrap().finish(),
                NativeList::new(&ledger, 0).unwrap().finish(),
                NativeOwned::supplied(arguments),
                NativeOwned::supplied(crate::model::native::content::null_json(&ledger).unwrap()),
                NativeOwned::absent(&ledger).unwrap(),
                NativeOwned::absent(&ledger).unwrap(),
            )
            .unwrap(),
        );
        let entries = vec![
            crate::history::native::entries::user(content.finish()),
            crate::history::native::entries::thought(text(&ledger, &"thought".repeat(11))),
            crate::history::native::entries::agent(text(&ledger, &"reply".repeat(61))),
            tool,
        ];
        // Independent old DTO oracle only; actual physical build consumes the
        // original managed inputs and never materializes this clone in production.
        let oracle = entries
            .iter()
            .map(|value| value.get().clone())
            .collect::<Vec<_>>();
        let mut builder = SnapshotBuilder::from_role(&mut foreground, &live).unwrap();
        for entry in entries {
            builder.append_row(entry, 7).unwrap();
        }
        live.replace(builder.finish(None).unwrap()).unwrap();
        for used in [0, 1, 50, 110, 10_000] {
            assert_eq!(
                Breakdown::estimate_managed(live.snapshot().unwrap().rows(), used).unwrap(),
                Breakdown::estimate(&oracle, used)
            );
        }
    }
}
