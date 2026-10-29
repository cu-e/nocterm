use crate::RunnerError;
use nocterm_cloud_model::{
    identity::{RunnerId, RunnerKeyEpoch},
    journal::JournalCatalogBinding,
};
use nocterm_core::SharedCause;
use nocterm_hub_io::cloud::RunnerHubTrust;
use nocterm_hubs_model::{Fingerprint, HubAddress, HubEndpointSpec, TrustPolicy};
use serde::Deserialize;
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
    time::Duration,
};

const MAX_CONFIG_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    hub: Hub,
    account: Account,
    runner: Assignment,
    state: PathBuf,
    #[serde(default)]
    limits: ControlLimits,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hub {
    address: String,
    instance: String,
    authority: String,
    tls_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    id: String,
    public_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assignment {
    id: String,
    key_epoch: u64,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ControlLimits {
    pub(crate) database_bytes: u64,
    pub(crate) chat_bytes: u64,
    pub(crate) records_per_chat: u64,
    pub(crate) chats: u64,
    pub(crate) operation_ms: u64,
    pub(crate) shutdown_ms: u64,
    pub(crate) head_wait_ms: u64,
}
impl Default for ControlLimits {
    fn default() -> Self {
        Self {
            database_bytes: 64 * 1024 * 1024,
            chat_bytes: 8 * 1024 * 1024,
            records_per_chat: 100_000,
            chats: 64,
            operation_ms: 15_000,
            shutdown_ms: 5_000,
            head_wait_ms: 500,
        }
    }
}
impl ControlLimits {
    fn validate(&self) -> Result<(), RunnerError> {
        if self.operation_ms == 0
            || self.operation_ms > 60_000
            || self.shutdown_ms == 0
            || self.shutdown_ms > 60_000
            || self.head_wait_ms == 0
            || self.head_wait_ms > 5_000
        {
            return Err(RunnerError::Value("limits"));
        }
        if self.chats > 1024 {
            return Err(RunnerError::Value("limits.chats"));
        }
        nocterm_cloud_runner::JournalLimits::new(
            self.database_bytes,
            self.chat_bytes,
            self.records_per_chat,
            self.chats,
        )?;
        Ok(())
    }
    pub(crate) fn operation(&self) -> Duration {
        Duration::from_millis(self.operation_ms)
    }
    pub(crate) fn shutdown(&self) -> Duration {
        Duration::from_millis(self.shutdown_ms)
    }
}
pub(crate) struct Configuration {
    pub(crate) trust: RunnerHubTrust,
    pub(crate) binding: JournalCatalogBinding,
    pub(crate) account: [u8; 32],
    pub(crate) state: PathBuf,
    pub(crate) limits: ControlLimits,
}
impl Configuration {
    pub(crate) fn read(path: &Path) -> Result<Self, RunnerError> {
        let mut file = nocterm_core::fs::open_private_regular(path).map_err(configuration)?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(configuration)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(RunnerError::Value("configuration size"));
        }
        let text = std::str::from_utf8(&bytes).map_err(configuration)?;
        let document: Document = toml::from_str(text).map_err(configuration)?;
        Self::prepare(document)
    }
    fn prepare(document: Document) -> Result<Self, RunnerError> {
        document.limits.validate()?;
        if !document.state.is_absolute()
            || document
                .state
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(RunnerError::Value("state"));
        }
        let authority = fixed_hex(&document.hub.authority, "hub.authority")?;
        let account = fixed_hex(&document.account.public_key, "account.public_key")?;
        let runner = RunnerId::from_bytes(fixed_hex(&document.runner.id, "runner.id")?);
        let epoch = RunnerKeyEpoch::new(document.runner.key_epoch)
            .map_err(|_| RunnerError::Value("runner.key_epoch"))?;
        let address = HubAddress::parse(&document.hub.address)
            .map_err(|_| RunnerError::Value("hub.address"))?;
        let trust = match document.hub.tls_sha256 {
            Some(value) => TrustPolicy::Pinned(Fingerprint::from_bytes(fixed_hex(
                &value,
                "hub.tls_sha256",
            )?)),
            None => TrustPolicy::PublicCa,
        };
        let trust = RunnerHubTrust::new(
            HubEndpointSpec { address, trust },
            document.hub.instance,
            authority,
        );
        let binding = trust.binding(&document.account.id, account, runner, epoch)?;
        Ok(Self {
            trust,
            binding,
            account,
            state: document.state,
            limits: document.limits,
        })
    }
}
fn configuration(error: impl std::error::Error + Send + Sync + 'static) -> RunnerError {
    RunnerError::Configuration(SharedCause::new(error))
}
fn fixed_hex<const N: usize>(text: &str, field: &'static str) -> Result<[u8; N], RunnerError> {
    if text.len() != 2 * N {
        return Err(RunnerError::Value(field));
    }
    let mut result = [0; N];
    for (out, pair) in result.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let high = digit(pair[0]).ok_or(RunnerError::Value(field))?;
        let low = digit(pair[1]).ok_or(RunnerError::Value(field))?;
        *out = high * 16 + low;
    }
    Ok(result)
}
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn fixture(state: PathBuf) -> Configuration {
    Configuration::prepare(Document {
        hub: Hub {
            address: "https://localhost:8443".into(),
            instance: "runner-test-hub".into(),
            authority: "05".repeat(32),
            tls_sha256: None,
        },
        account: Account {
            id: "runner-test-account".into(),
            public_key: "06".repeat(32),
        },
        runner: Assignment {
            id: "07".repeat(16),
            key_epoch: 1,
        },
        state,
        limits: ControlLimits::default(),
    })
    .unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runner_configuration_refuses_invalid_limits_before_native_creation() {
        let mut limits = ControlLimits {
            chats: 1025,
            ..ControlLimits::default()
        };
        assert!(limits.validate().is_err());
        limits.chats = 1;
        limits.chat_bytes = limits.database_bytes + 1;
        assert!(limits.validate().is_err());
        assert!(fixed_hex::<32>(&"ff".repeat(31), "pin").is_err());
        assert!(fixed_hex::<32>(&"gg".repeat(32), "pin").is_err());
    }
}
