//! `[vault]`: when the encrypted vault locks and asks for its password.

use nocterm_settings::SettingsSection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Automatic locking of the portable encrypted vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct VaultSettings {
    /// Lock after this many minutes without vault use (1–1440).
    #[schemars(extend("minimum" = 1, "maximum" = 1440))]
    pub auto_lock_minutes: u32,
    /// Prompt for the master password when Nocterm starts.
    pub prompt_on_startup: bool,
}
impl Default for VaultSettings {
    fn default() -> Self {
        Self {
            auto_lock_minutes: 15,
            prompt_on_startup: false,
        }
    }
}

impl SettingsSection for VaultSettings {
    const KEY: &'static str = "vault";

    fn sanitize(&mut self) {
        self.auto_lock_minutes = self.auto_lock_minutes.clamp(1, 1440);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_clamps_the_lock_delay_and_leaves_the_defaults_alone() {
        let mut vault = VaultSettings {
            auto_lock_minutes: 0,
            ..VaultSettings::default()
        };
        vault.sanitize();
        assert_eq!(vault.auto_lock_minutes, 1);
        let mut defaults = VaultSettings::default();
        defaults.sanitize();
        assert_eq!(defaults, VaultSettings::default());
    }
}
