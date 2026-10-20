//! Claude Code through Zed's `claude-agent-acp` adapter.
use nocterm_ai::agents::{
    AccountUsage, AgentMark, AgentProfile, LaunchDefaults, UsageMeta, UsageProfile,
};

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
    UsageProfile::new(
        UsageMeta::RateLimitRecord {
            key: "_claude/rateLimit",
        },
        AccountUsage::OAuthAccount,
    )
    .with_empty_hint("Claude plan limits load from your Claude Code sign-in."),
)
.with_mark(
    AgentMark::new(
        "icons/agents/claude.svg",
        include_bytes!("../assets/claude.svg"),
    )
    .tinted(0xd97757),
);
