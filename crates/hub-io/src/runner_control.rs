//! The host-to-runner chat access frames of the public contract (contract ADR 0003,
//! desktop ADR 0123), re-exported for the runner: only this adapter depends on the
//! hub contract, so the runner reads its control channel through these names.

pub use nocterm_hub_model::agents::{
    ChatAccessDecoder, ChatAccessFrame, ChatAccessFrameError, ChatAccessToken,
};

/// The bounds a managed agent policy is checked against (contract ADR 0004).
pub mod limits {
    pub use nocterm_hub_model::agents::limits::{
        MAX_AGENT_OUTPUT_TOKENS, MAX_AGENT_TURNS, MAX_TOOL_NAME_BYTES, MAX_TOOL_POLICY_ENTRIES,
    };
}

/// The SSH section of a managed boot document: version, targets with their optional host
/// keys and session settings, and egress (contract ADR 0011, desktop ADR 0139).
pub mod boot_ssh {
    pub use nocterm_hub_model::agents::boot_ssh::{
        BOOT_VERSION_SSH, BOOT_VERSION_WITHOUT_SSH_EGRESS, BootSshError, BootSshField,
        BootSshSession, BootSshTarget, check_boot_ssh,
    };
    pub use nocterm_hub_model::cloud::ssh::is_source_address;
}
