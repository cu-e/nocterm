//! The agent definition v2 policy a boot v2 document may carry (ADR 0127):
//! which tools the model is offered, when a person approves an effectful call
//! and how far a run may go. It only ever narrows what the operator and the
//! command already allow.
use crate::RunnerError;
use nocterm_cloud_model::engine::{EngineError, ModelBudget};
use nocterm_hub_model::agents::limits::{
    MAX_AGENT_OUTPUT_TOKENS, MAX_AGENT_TURNS, MAX_TOOL_NAME_BYTES, MAX_TOOL_POLICY_ENTRIES,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(untagged)]
enum Allowed {
    Keyword(String),
    Names(Vec<String>),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tools {
    allowed: Allowed,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Approval {
    Never,
    #[default]
    Ask,
    Always,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    max_turns: Option<u64>,
    max_output_tokens: Option<u64>,
    max_spend_micros_per_run: Option<u64>,
    max_spend_micros_per_chat: Option<u64>,
}
/// The wire form, exactly as the host renders it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyDocument {
    tools: Tools,
    #[serde(default)]
    approval: Approval,
    #[serde(default)]
    limits: Limits,
}

/// A validated policy. The default is today's behaviour: every tool, the
/// existing approval rule and no extra limits.
#[derive(Clone, Debug, Default)]
pub(crate) struct ManagedPolicy {
    /// `None` offers every tool.
    tools: Option<BTreeSet<String>>,
    approval: Approval,
    max_turns: Option<u32>,
    max_output_tokens: Option<u64>,
    spend_per_run: Option<u64>,
    spend_per_chat: Option<u64>,
}
fn bounded(value: Option<u64>, maximum: usize) -> Result<Option<u64>, RunnerError> {
    match value {
        Some(value) if value == 0 || value > maximum as u64 => {
            Err(RunnerError::Value("boot.policy.limits"))
        }
        other => Ok(other),
    }
}
impl ManagedPolicy {
    pub(super) fn from_document(document: PolicyDocument) -> Result<Self, RunnerError> {
        let tools = match document.tools.allowed {
            Allowed::Keyword(word) if word == "all" => None,
            Allowed::Keyword(_) => return Err(RunnerError::Value("boot.policy.tools")),
            Allowed::Names(names) => {
                if names.len() > MAX_TOOL_POLICY_ENTRIES
                    || names
                        .iter()
                        .any(|name| name.is_empty() || name.len() > MAX_TOOL_NAME_BYTES)
                {
                    return Err(RunnerError::Value("boot.policy.tools"));
                }
                Some(names.into_iter().collect())
            }
        };
        let limits = document.limits;
        let positive = |value: Option<u64>| match value {
            Some(0) => Err(RunnerError::Value("boot.policy.limits")),
            other => Ok(other),
        };
        let spend_per_run = positive(limits.max_spend_micros_per_run)?;
        let spend_per_chat = positive(limits.max_spend_micros_per_chat)?;
        if let (Some(run), Some(chat)) = (spend_per_run, spend_per_chat)
            && run > chat
        {
            return Err(RunnerError::Value("boot.policy.limits"));
        }
        Ok(Self {
            tools,
            approval: document.approval,
            max_turns: bounded(limits.max_turns, MAX_AGENT_TURNS)?
                .map(|turns| u32::try_from(turns).unwrap_or(u32::MAX)),
            max_output_tokens: bounded(limits.max_output_tokens, MAX_AGENT_OUTPUT_TOKENS)?,
            spend_per_run,
            spend_per_chat,
        })
    }
    pub(crate) fn approval(&self) -> Approval {
        self.approval
    }
    /// The names the model may be offered; `None` means the whole catalog.
    pub(crate) fn allowed_tools(&self) -> Option<&BTreeSet<String>> {
        self.tools.as_ref()
    }
    pub(crate) fn spend_per_run(&self) -> Option<u64> {
        self.spend_per_run
    }
    pub(crate) fn spend_per_chat(&self) -> Option<u64> {
        self.spend_per_chat
    }
    /// The command's budget, lowered to the policy. A policy never raises it.
    pub(crate) fn narrow(&self, budget: ModelBudget) -> Result<ModelBudget, EngineError> {
        let turns = self
            .max_turns
            .map_or(budget.turns(), |max| max.min(budget.turns()));
        let tokens = self
            .max_output_tokens
            .map_or(budget.output_tokens(), |max| {
                max.min(budget.output_tokens())
            });
        ModelBudget::new(turns, budget.input_bytes(), budget.output_bytes(), tokens)?
            .with_calls(budget.calls().min(turns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(value: serde_json::Value) -> Result<ManagedPolicy, RunnerError> {
        let document: PolicyDocument =
            serde_json::from_value(value).map_err(|_| RunnerError::Value("boot.policy"))?;
        ManagedPolicy::from_document(document)
    }
    #[test]
    fn a_full_policy_is_read_as_the_host_renders_it() {
        let policy = parse(serde_json::json!({
            "tools": {"allowed": ["read_terminal", "no_such_tool"]}, "approval": "always",
            "limits": {"max_turns": 5, "max_output_tokens": 100,
                "max_spend_micros_per_run": 10, "max_spend_micros_per_chat": 20}
        }))
        .unwrap();
        assert_eq!(policy.approval(), Approval::Always);
        assert_eq!(policy.allowed_tools().unwrap().len(), 2);
        assert_eq!(
            (policy.spend_per_run(), policy.spend_per_chat()),
            (Some(10), Some(20))
        );
        let all = parse(serde_json::json!({"tools": {"allowed": "all"}, "approval": "never"}));
        assert!(all.unwrap().allowed_tools().is_none());
    }
    #[test]
    fn invalid_or_oversize_policies_are_refused() {
        let names = |count: usize, len: usize| {
            let list = (0..count)
                .map(|i| format!("{i:0>len$}"))
                .collect::<Vec<_>>();
            serde_json::json!({"tools": {"allowed": list}})
        };
        assert!(parse(names(MAX_TOOL_POLICY_ENTRIES, 1)).is_ok());
        assert!(parse(names(MAX_TOOL_POLICY_ENTRIES + 1, 3)).is_err());
        assert!(parse(names(1, MAX_TOOL_NAME_BYTES + 1)).is_err());
        assert!(parse(serde_json::json!({"tools": {"allowed": [""]}})).is_err());
        let limit = |key: &str, value: u64| serde_json::json!({"tools": {"allowed": "all"}, "limits": {key: value}});
        assert!(parse(limit("max_turns", 0)).is_err());
        assert!(parse(limit("max_turns", MAX_AGENT_TURNS as u64 + 1)).is_err());
        assert!(
            parse(limit(
                "max_output_tokens",
                MAX_AGENT_OUTPUT_TOKENS as u64 + 1
            ))
            .is_err()
        );
        assert!(parse(limit("max_spend_micros_per_run", 0)).is_err());
        for bad in [
            serde_json::json!({"tools": {"allowed": "some"}}),
            serde_json::json!({"tools": {"allowed": "all"}, "approval": "maybe"}),
            serde_json::json!({"tools": {"allowed": "all"}, "extra": 1}),
            serde_json::json!({"approval": "ask"}),
            serde_json::json!({"tools": {"allowed": "all"}, "limits": {
                "max_spend_micros_per_run": 9, "max_spend_micros_per_chat": 8}}),
        ] {
            assert!(parse(bad).is_err());
        }
    }
    #[test]
    fn limits_only_narrow_the_budget_of_the_command() {
        let budget = ModelBudget::new(8, 4096, 4096, 500).unwrap();
        let policy = |turns, tokens| {
            parse(serde_json::json!({"tools": {"allowed": "all"},
                "limits": {"max_turns": turns, "max_output_tokens": tokens}}))
            .unwrap()
        };
        let narrowed = policy(3, 100).narrow(budget).unwrap();
        assert_eq!((narrowed.turns(), narrowed.output_tokens()), (3, 100));
        assert_eq!(narrowed.calls(), 3);
        let wider = policy(50, 9000).narrow(budget).unwrap();
        assert_eq!((wider.turns(), wider.output_tokens()), (8, 500));
        let none = ManagedPolicy::default().narrow(budget).unwrap();
        assert_eq!((none.turns(), none.output_tokens()), (8, 500));
    }
}
