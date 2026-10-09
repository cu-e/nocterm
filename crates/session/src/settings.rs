//! `[ssh]` and `[local]`: how connections are made and shells are launched.
//!
//! The doc comment of every field doubles as its entry in the generated
//! reference (`docs/reference/settings.md`) and as its description in the
//! settings tab, so write it for the person changing the setting.

use std::{collections::BTreeMap, ops::RangeInclusive};

use nocterm_settings::SettingsSection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ProxyConfig;

/// Connection timeouts a user may pick, in seconds.
pub const CONNECT_TIMEOUT_RANGE: RangeInclusive<u32> = 1..=600;
/// Keep-alive intervals a user may pick, in seconds; zero turns them off.
pub const KEEPALIVE_RANGE: RangeInclusive<u32> = 0..=3600;

/// How SSH connections are made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SshSettings {
    /// Explicit connection route; proxy authentication is not supported yet.
    pub proxy: ProxyConfig,
    /// Seconds to wait for a host to answer before giving up.
    #[schemars(extend("minimum" = CONNECT_TIMEOUT_RANGE.start(), "maximum" = CONNECT_TIMEOUT_RANGE.end()))]
    pub connect_timeout_secs: u32,
    /// Seconds between keep-alive probes on an idle connection. 0 turns them off.
    #[schemars(extend("minimum" = KEEPALIVE_RANGE.start(), "maximum" = KEEPALIVE_RANGE.end()))]
    pub keepalive_interval_secs: u32,
    /// Default remote shell launch options; profiles may override them.
    pub launch: ShellSettings,
}

impl Default for SshSettings {
    fn default() -> Self {
        Self {
            proxy: ProxyConfig::default(),
            connect_timeout_secs: 15,
            keepalive_interval_secs: 30,
            launch: ShellSettings::default(),
        }
    }
}

/// Shell launch settings. Changes apply on the next launch or reconnect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ShellSettings {
    /// Shell executable. Empty means the system or server default.
    pub program: Option<String>,
    /// Individual executable arguments, without shell parsing.
    pub args: Vec<String>,
    /// Initial directory. Empty means the user's home directory.
    pub cwd: Option<String>,
    /// Additional environment variables; never put passwords here.
    pub env: BTreeMap<String, String>,
    /// Enable ephemeral cwd and prompt integration in supported shells.
    pub integration: bool,
}
impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            program: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            integration: true,
        }
    }
}

impl SettingsSection for SshSettings {
    const KEY: &'static str = "ssh";

    fn sanitize(&mut self) {
        self.connect_timeout_secs = self
            .connect_timeout_secs
            .clamp(*CONNECT_TIMEOUT_RANGE.start(), *CONNECT_TIMEOUT_RANGE.end());
        self.keepalive_interval_secs = self
            .keepalive_interval_secs
            .clamp(*KEEPALIVE_RANGE.start(), *KEEPALIVE_RANGE.end());
    }
}

/// Local shell launch options; applied when opening the bottom terminal.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LocalShellSettings(pub ShellSettings);

impl std::ops::Deref for LocalShellSettings {
    type Target = ShellSettings;

    fn deref(&self) -> &ShellSettings {
        &self.0
    }
}

impl std::ops::DerefMut for LocalShellSettings {
    fn deref_mut(&mut self) -> &mut ShellSettings {
        &mut self.0
    }
}

impl SettingsSection for LocalShellSettings {
    const KEY: &'static str = "local";
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_settings::SettingsDocument;

    fn read<S: SettingsSection>(text: &str) -> S {
        let mut document = SettingsDocument::from_table(toml::from_str(text).unwrap());
        document.register::<S>();
        assert!(document.errors().is_empty(), "{:?}", document.errors());
        document.get::<S>().clone()
    }

    #[test]
    fn sanitizing_clamps_the_timeouts() {
        let mut ssh = SshSettings {
            connect_timeout_secs: 0,
            ..SshSettings::default()
        };
        ssh.sanitize();
        assert_eq!(ssh.connect_timeout_secs, 1);
    }

    #[test]
    fn the_local_shell_is_a_plain_table() {
        let local: LocalShellSettings = read("[local]\nargs = [\"-l\"]\n");
        assert_eq!(local.args, ["-l"]);
        assert!(local.integration);
    }

    #[test]
    fn sanitizing_leaves_the_defaults_alone() {
        let mut ssh = SshSettings::default();
        ssh.sanitize();
        assert_eq!(ssh, SshSettings::default());
        let mut local = LocalShellSettings::default();
        local.sanitize();
        assert_eq!(local, LocalShellSettings::default());
    }
}
