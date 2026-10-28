//! Claude Code through Zed's `claude-agent-acp` adapter.
use nocterm_ai::agents::{AgentMark, AgentProfile, LaunchDefaults, UsageMeta, UsageProfile};

pub(crate) const PROFILE: AgentProfile = AgentProfile::new(
    "claude",
    "Claude",
    LaunchDefaults::new(
        "npx",
        &["-y", "@agentclientprotocol/claude-agent-acp@0.85.1"],
        &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CONFIG_DIR",
        ],
    ),
)
.recognized_by(&["claude-agent-acp"])
.writes_home(&[".claude", ".claude.json", ".config/claude"])
.with_usage(
    UsageProfile::new(UsageMeta::RateLimitRecord {
        key: "_claude/rateLimit",
    })
    .with_empty_hint("Claude has not reported plan limits."),
)
.with_mark(
    AgentMark::new(
        "icons/agents/claude.svg",
        include_bytes!("../assets/claude.svg"),
    )
    .tinted(0xd97757),
);
