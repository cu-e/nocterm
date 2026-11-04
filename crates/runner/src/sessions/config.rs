//! Explicit administrator routes; protocol fingerprints never become endpoints.
#[cfg(target_os = "linux")]
use super::targets::{ConfiguredSessionTarget, ConfiguredSessionTargets};
use crate::{RunnerError, config::fixed_hex};
#[cfg(target_os = "linux")]
use nocterm_cloud_model::journal::JournalCatalogBinding;
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, TargetBinding},
    coordinator::RunDestination,
    identity::{Fingerprint, NamespaceId},
};
#[cfg(target_os = "linux")]
use nocterm_local::ContainedLocal;
use nocterm_session::ShellLaunch;
#[cfg(target_os = "linux")]
use nocterm_session::{Auth, ConnectRequest, ProxyConfig, PtySize, Target};
#[cfg(target_os = "linux")]
use nocterm_ssh::SshTransport;
use nocterm_ssh_keys::PublicKeyText;
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::{sync::Arc, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionTargetConfiguration {
    default: String,
    targets: Vec<TargetDocument>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetDocument {
    name: String,
    host_fingerprint: String,
    route_fingerprint: String,
    principal: String,
    workspace_fingerprint: String,
    authentication_key: String,
    credential_revision: String,
    connector: ConnectorDocument,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ConnectorDocument {
    Local {
        launch: ShellLaunch,
        terminal: String,
        columns: u16,
        rows: u16,
    },
    Ssh {
        host: String,
        port: u16,
        host_public_key: String,
        launch: ShellLaunch,
        terminal: String,
        columns: u16,
        rows: u16,
        timeout_ms: u64,
    },
}
impl SessionTargetConfiguration {
    fn validate(&self) -> Result<(), RunnerError> {
        if self.default.is_empty()
            || self.default.len() > 128
            || self.targets.is_empty()
            || self.targets.len() > 128
        {
            return Err(RunnerError::Value("session_targets"));
        }
        for (index, entry) in self.targets.iter().enumerate() {
            if entry.name.is_empty()
                || entry.name.len() > 128
                || self.targets[..index]
                    .iter()
                    .any(|old| old.name == entry.name)
            {
                return Err(RunnerError::Value("session_targets.name"));
            }
            let (target, _) = entry.bindings()?;
            entry.connector.validate(&target)?;
        }
        Ok(())
    }
    /// A configured named default, with no pre-login receipt or dummy SSH key.
    pub(crate) fn default_destination(&self) -> Result<RunDestination, RunnerError> {
        self.validate()?;
        let selected = self
            .targets
            .iter()
            .find(|entry| entry.name == self.default)
            .ok_or(RunnerError::Value("session_targets.default"))?;
        let (target, authentication) = selected.bindings()?;
        Ok(RunDestination::new(target, authentication, None)?)
    }
    /// The configured SSH targets as endpoints with their host key fingerprints.
    pub(crate) fn ssh_endpoints(
        &self,
    ) -> Result<Vec<(nocterm_cloud_model::ssh_access::SshEndpoint, Fingerprint)>, RunnerError> {
        self.validate()?;
        let mut endpoints = Vec::new();
        for entry in &self.targets {
            if let ConnectorDocument::Ssh { host, port, .. } = &entry.connector {
                let (target, _) = entry.bindings()?;
                let endpoint = nocterm_cloud_model::ssh_access::SshEndpoint {
                    host: host.clone(),
                    port: *port,
                    user: entry.principal.clone(),
                };
                endpoints.push((endpoint, target.host()));
            }
        }
        Ok(endpoints)
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn build(
        &self,
        binding: JournalCatalogBinding,
        local: &ContainedLocal,
        ssh: Arc<SshTransport>,
        credentials: Arc<dyn nocterm_cloud_model::ssh_access::ChatSshCredentials>,
    ) -> Result<Arc<ConfiguredSessionTargets>, RunnerError> {
        self.default_destination()?;
        let entries = self
            .targets
            .iter()
            .map(|entry| entry.build(local, ssh.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Arc::new(
            ConfiguredSessionTargets::new(binding, entries)?.with_credentials(credentials),
        ))
    }
}
impl ConnectorDocument {
    fn validate(&self, target: &TargetBinding) -> Result<(), RunnerError> {
        let (launch, terminal, columns, rows) = match self {
            Self::Local {
                launch,
                terminal,
                columns,
                rows,
            } => (launch, terminal, columns, rows),
            Self::Ssh {
                host,
                port,
                host_public_key,
                launch,
                terminal,
                columns,
                rows,
                timeout_ms,
            } => {
                if host.is_empty()
                    || host.len() > 253
                    || *port == 0
                    || *timeout_ms == 0
                    || *timeout_ms > 60_000
                    || host_public_key.len() > 2048
                {
                    return Err(RunnerError::Value("session_targets.ssh"));
                }
                let host_key = PublicKeyText::parse(host_public_key)
                    .map_err(|_| RunnerError::Value("session_targets.host_public_key"))?;
                if host_key
                    .fingerprint_bytes()
                    .map_err(|_| RunnerError::Value("session_targets.host_public_key"))?
                    != *target.host().as_bytes()
                {
                    return Err(RunnerError::Value("session_targets.host_fingerprint"));
                }
                (launch, terminal, columns, rows)
            }
        };
        let strings = launch
            .program
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(launch.cwd.as_ref().map_or(0, String::len))
            .saturating_add(launch.args.iter().map(String::len).sum::<usize>());
        if launch.validate().is_err()
            || !launch.env.is_empty()
            || launch.args.len() > 64
            || strings > 16 * 1024
            || terminal.is_empty()
            || terminal.len() > 128
            || !terminal
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte))
            || !(2..=240).contains(columns)
            || !(1..=120).contains(rows)
            || matches!(self, Self::Local { .. }) && !launch.integration
        {
            return Err(RunnerError::Value("session_targets.launch"));
        }
        Ok(())
    }
}
impl TargetDocument {
    fn bindings(&self) -> Result<(TargetBinding, AuthenticationBinding), RunnerError> {
        let fp = |value: &str| {
            fixed_hex(value, "session_targets.fingerprint").map(Fingerprint::from_bytes)
        };
        let principal = NamespaceId::new(self.principal.clone())
            .map_err(|_| RunnerError::Value("session_targets.principal"))?;
        Ok((
            TargetBinding::new(
                fp(&self.host_fingerprint)?,
                fp(&self.route_fingerprint)?,
                principal,
                fp(&self.workspace_fingerprint)?,
            ),
            AuthenticationBinding::new(
                fp(&self.authentication_key)?,
                fp(&self.credential_revision)?,
            ),
        ))
    }
    #[cfg(target_os = "linux")]
    fn build(
        &self,
        local: &ContainedLocal,
        ssh: Arc<SshTransport>,
    ) -> Result<ConfiguredSessionTarget, RunnerError> {
        let (target, authentication) = self.bindings()?;
        match &self.connector {
            ConnectorDocument::Local {
                launch,
                terminal,
                columns,
                rows,
            } => Ok(ConfiguredSessionTarget::local(
                target,
                authentication,
                local.clone(),
                launch.clone(),
                terminal.clone(),
                size(*columns, *rows),
            )?),
            ConnectorDocument::Ssh {
                host,
                port,
                host_public_key,
                launch,
                terminal,
                columns,
                rows,
                timeout_ms,
            } => {
                let host_key = PublicKeyText::parse(host_public_key)
                    .map_err(|_| RunnerError::Value("session_targets.host_public_key"))?;
                let request = ConnectRequest {
                    proxy: ProxyConfig::Direct,
                    launch: launch.clone(),
                    target: Target {
                        host: host.clone(),
                        port: *port,
                        user: self.principal.clone(),
                    },
                    auth: Auth::Auto,
                    jumps: Vec::new(),
                    term: terminal.clone(),
                    size: size(*columns, *rows),
                    connect_timeout: Duration::from_millis(*timeout_ms),
                    keepalive_interval: Some(Duration::from_secs(30)),
                };
                Ok(ConfiguredSessionTarget::ssh(
                    target,
                    authentication,
                    ssh,
                    request,
                    host_key,
                )?)
            }
        }
    }
}
#[cfg(target_os = "linux")]
fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}
