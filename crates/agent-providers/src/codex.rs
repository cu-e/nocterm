//! OpenAI Codex through the `codex-acp` adapter.
use nocterm_ai::agents::{
    AccountUsage, AgentMark, AgentProfile, LaunchDefaults, UsageMeta, UsageProfile,
};

pub(crate) const PROFILE: AgentProfile = AgentProfile::new(
    "codex",
    "Codex",
    LaunchDefaults::new(
        "npx",
        &["-y", "@agentclientprotocol/codex-acp@2.1.1"],
        &["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_HOME"],
    ),
)
.recognized_by(&["codex"])
.writes_home(&[".codex"])
.with_usage(
    UsageProfile::new(UsageMeta::None, AccountUsage::SessionLog)
        .with_empty_hint("Codex reports plan limits after a reply."),
)
.with_mark(AgentMark::new(
    "icons/agents/codex.svg",
    include_bytes!("../assets/codex.svg"),
));
