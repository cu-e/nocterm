//! Session admission and process container limits are independent of chat history.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSessionSettings {
    /// Maximum starting, live and closing sessions, shared by every window.
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub max_live: usize,
    /// Maximum warm idle sessions. Zero releases every idle session.
    #[schemars(extend("minimum" = 0, "maximum" = 64))]
    pub max_idle: usize,
    /// Release a session after this many idle seconds.
    #[schemars(extend("minimum" = 1, "maximum" = 3600))]
    pub idle_timeout_secs: u64,
}
impl Default for AgentSessionSettings {
    fn default() -> Self {
        Self {
            max_live: 4,
            max_idle: 2,
            idle_timeout_secs: 90,
        }
    }
}
impl AgentSessionSettings {
    pub(crate) fn sanitize(&mut self) {
        self.max_live = self.max_live.clamp(1, 64);
        self.max_idle = self.max_idle.min(self.max_live);
        self.idle_timeout_secs = self.idle_timeout_secs.clamp(1, 3600);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentResourceSettings {
    /// Memory pressure threshold for each agent tree, in MiB.
    #[schemars(extend("minimum" = 64, "maximum" = 1048576))]
    pub memory_high_mb: u64,
    /// Hard memory limit for each agent tree, in MiB.
    #[schemars(extend("minimum" = 64, "maximum" = 1048576))]
    pub memory_max_mb: u64,
    /// Maximum swap used by each agent tree, in MiB. Zero disables swap.
    #[schemars(extend("minimum" = 0, "maximum" = 1048576))]
    pub memory_swap_max_mb: u64,
    /// Maximum processes and threads in each agent tree.
    #[schemars(extend("minimum" = 16, "maximum" = 65536))]
    pub tasks_max: u64,
}
impl Default for AgentResourceSettings {
    fn default() -> Self {
        Self {
            memory_high_mb: 2048,
            memory_max_mb: 4096,
            memory_swap_max_mb: 1024,
            tasks_max: 512,
        }
    }
}
impl AgentResourceSettings {
    pub(crate) fn sanitize(&mut self) {
        self.memory_max_mb = self.memory_max_mb.clamp(64, 1048576);
        self.memory_high_mb = self.memory_high_mb.clamp(64, self.memory_max_mb);
        self.memory_swap_max_mb = self.memory_swap_max_mb.min(1048576);
        self.tasks_max = self.tasks_max.clamp(16, 65536);
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut sanitized = self.clone();
        sanitized.sanitize();
        if sanitized == *self {
            Ok(())
        } else {
            Err("Invalid agent resource limits.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Settings, SettingsFile};
    #[test]
    fn global_admission_and_memory_limits_are_bounded_and_round_trip() {
        let mut settings = Settings::default();
        settings.ai.sessions = AgentSessionSettings {
            max_live: 0,
            max_idle: usize::MAX,
            idle_timeout_secs: 0,
        };
        settings.ai.resources = AgentResourceSettings {
            memory_high_mb: u64::MAX,
            memory_max_mb: 0,
            memory_swap_max_mb: u64::MAX,
            tasks_max: 0,
        };
        let settings = settings.sanitized();
        assert_eq!(
            settings.ai.sessions,
            AgentSessionSettings {
                max_live: 1,
                max_idle: 1,
                idle_timeout_secs: 1
            }
        );
        assert_eq!(settings.ai.resources.memory_high_mb, 64);
        assert_eq!(settings.ai.resources.memory_max_mb, 64);
        assert_eq!(settings.ai.resources.tasks_max, 16);
        assert!(settings.ai.resources.validate().is_ok());
        let dir = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(dir.path().join("settings.toml"));
        file.save(&settings).unwrap();
        assert_eq!(file.load().unwrap().settings, settings);
    }
    #[test]
    fn unsanitized_process_limits_fail_validation() {
        let resources = AgentResourceSettings {
            memory_high_mb: 4096,
            memory_max_mb: 1024,
            ..Default::default()
        };
        assert!(resources.validate().is_err());
    }
}
