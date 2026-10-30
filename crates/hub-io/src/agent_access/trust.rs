//! The operations on the registry: allow, deny, reconcile, reissue and overview.
//!
//! Every change that depends on a server first records the intent in the account's own
//! collection and only then edits the server, so a server that cannot be reached is never
//! forgotten: a denial stays `RemovalPending` and is finished on the next connection.

use std::future::Future;
use std::sync::Arc;

use nocterm_hubs_model::{
    AgentAccessError, AgentCa, AgentTrustOverview, HostChoice, HubId, ServerTrust, TrustedServer,
};
use nocterm_keys::agents::{
    CaLine, Observation, PRINCIPAL_PREFIX, SshTrustRecord, TrustAction, TrustState, find_ca_lines,
    remove_ca_lines,
};
use nocterm_keys::random_bytes;
use nocterm_ssh_keys::authorized_keys::{AgentHosts, HostAccess, HostAccessError, with_line};
use sha2::{Digest, Sha256};

use super::AccountWork;
use super::document::CaDocument;
use super::registry::{Registry, records};
use crate::account::operations::OpenedWork;

/// Milliseconds since the Unix epoch.
pub(super) type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

const LINE_COMMENT: &str = "nocterm agent access";

/// The principal a CA line admits: derived from the CA key, so a line and its certificates
/// agree without the server knowing the account.
pub(super) fn principal_of(ca_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(ca_key);
    let mut principal = String::from(PRINCIPAL_PREFIX);
    for byte in &digest[..8] {
        principal.push_str(&format!("{byte:02x}"));
    }
    principal
}

pub(super) fn line_of(document: &CaDocument) -> Result<String, AgentAccessError> {
    let ca_key = document.ca().public_key();
    CaLine {
        ca_key,
        principals: vec![principal_of(&ca_key)],
        comment: LINE_COMMENT.to_owned(),
    }
    .to_line()
    .map_err(records)
}

fn host_error(error: HostAccessError) -> AgentAccessError {
    match error {
        HostAccessError::NotConnected => AgentAccessError::NotConnected {
            connection: String::new(),
        },
        HostAccessError::Changed => AgentAccessError::HostChanged,
        HostAccessError::TooLarge | HostAccessError::NotText => AgentAccessError::HostUnsupported,
        HostAccessError::Host(cause) => AgentAccessError::Host { cause },
    }
}

fn state_of(state: TrustState) -> ServerTrust {
    match state {
        TrustState::Trusted => ServerTrust::Trusted,
        TrustState::RemovalPending => ServerTrust::RemovalPending,
        TrustState::Removed => ServerTrust::Removed,
        TrustState::Drifted => ServerTrust::Drifted,
    }
}

fn observe(text: &str, document: &CaDocument) -> Observation {
    if !find_ca_lines(text, &document.ca().public_key()).is_empty() {
        Observation::CurrentCaPresent
    } else if document
        .retired()
        .iter()
        .any(|key| !find_ca_lines(text, key).is_empty())
    {
        Observation::StaleCaPresent
    } else {
        Observation::Absent
    }
}

fn without_lines(text: &str, document: &CaDocument) -> String {
    document.retired().iter().fold(
        remove_ca_lines(text, &document.ca().public_key()),
        |text, key| remove_ca_lines(&text, key),
    )
}

fn summary(record: &SshTrustRecord) -> TrustedServer {
    TrustedServer {
        connection: record.profile_id.clone(),
        user: record.user.clone(),
        host: record.host.clone(),
        state: state_of(record.state()),
        added_at_ms: record.added_at_ms,
        added_by_device: record.added_by_device.to_string(),
        last_verified_at_ms: record.last_verified_at_ms,
    }
}

fn ca_summary(document: &CaDocument) -> AgentCa {
    AgentCa {
        fingerprint: document.ca().fingerprint().to_owned(),
        created_at_ms: document.created_at_ms(),
    }
}

fn fresh_seed() -> Result<[u8; 32], AgentAccessError> {
    random_bytes::<32>().map_err(records)
}

pub(super) struct Trust {
    source: Arc<dyn AccountWork>,
    hosts: Arc<dyn AgentHosts>,
    clock: Clock,
}

impl Trust {
    pub(super) fn new(
        source: Arc<dyn AccountWork>,
        hosts: Arc<dyn AgentHosts>,
        clock: Clock,
    ) -> Self {
        Self {
            source,
            hosts,
            clock,
        }
    }

    pub(super) fn sessions(&self) -> Vec<HostChoice> {
        self.hosts
            .sessions()
            .into_iter()
            .map(|session| HostChoice {
                connection: session.connection,
                user: session.user,
                host: session.host,
            })
            .collect()
    }

    fn host(&self, connection: &str) -> Result<HostAccess, AgentAccessError> {
        self.hosts.access(connection).map_err(|error| match error {
            HostAccessError::NotConnected => AgentAccessError::NotConnected {
                connection: connection.to_owned(),
            },
            other => host_error(other),
        })
    }

    /// Runs `body` while the original account is current.
    async fn guarded<T>(
        work: &OpenedWork,
        body: impl Future<Output = Result<T, AgentAccessError>>,
    ) -> Result<T, AgentAccessError> {
        work.source().during(body).await?
    }

    pub(super) async fn overview(
        &self,
        hub: &HubId,
    ) -> Result<AgentTrustOverview, AgentAccessError> {
        let work = self.source.work(hub)?;
        let store = work.transient_store();
        let loaded = Self::guarded(&work, Registry::load(&store, false)).await?;
        Ok(
            loaded.map_or_else(AgentTrustOverview::default, |registry| AgentTrustOverview {
                ca: registry.ca.as_ref().map(|ca| ca_summary(&ca.value)),
                servers: registry
                    .servers
                    .values()
                    .map(|server| summary(&server.value))
                    .collect(),
            }),
        )
    }

    pub(super) async fn ca_line(&self, hub: &HubId) -> Result<String, AgentAccessError> {
        let work = self.source.work(hub)?;
        let store = work.transient_store();
        let registry = Self::guarded(&work, Registry::load(&store, false)).await?;
        let ca =
            registry
                .and_then(|registry| registry.ca)
                .ok_or_else(|| AgentAccessError::Unknown {
                    connection: String::new(),
                })?;
        line_of(&ca.value)
    }

    pub(super) async fn allow(
        &self,
        hub: &HubId,
        connection: &str,
    ) -> Result<(), AgentAccessError> {
        let host = self.host(connection)?;
        let device = self.source.device(hub).await?;
        let work = self.source.work(hub)?;
        let store = work.transient_store();
        Self::guarded(&work, async {
            let mut registry = Registry::load(&store, true)
                .await?
                .ok_or(AgentAccessError::Hub(
                    nocterm_hubs_model::HubError::NotFound,
                ))?;
            if registry.ca.is_none() {
                let document = CaDocument::new(fresh_seed()?, (self.clock)()).map_err(records)?;
                registry.save_ca(&store, document).await?;
            }
            let document = &registry.ca.as_ref().map(|ca| &ca.value).ok_or_else(|| {
                AgentAccessError::Unknown {
                    connection: connection.to_owned(),
                }
            })?;
            let line = line_of(document)?;
            let text = host.file.read().await.map_err(host_error)?;
            let updated = with_line(&without_lines(&text, document), &line);
            if updated != text {
                host.file.replace(text, updated).await.map_err(host_error)?;
            }
            let record = SshTrustRecord::installed(
                connection.to_owned(),
                host.identity.host.clone(),
                host.identity.user.clone(),
                host.identity.host_key_fingerprint.clone(),
                document.ca().fingerprint().to_owned(),
                (self.clock)(),
                device,
            )
            .map_err(records)?;
            registry.save_server(&store, record).await
        })
        .await
    }

    pub(super) async fn deny(
        &self,
        hub: &HubId,
        connection: &str,
    ) -> Result<ServerTrust, AgentAccessError> {
        let work = self.source.work(hub)?;
        let store = work.transient_store();
        Self::guarded(&work, async {
            let mut registry =
                Registry::load(&store, false)
                    .await?
                    .ok_or_else(|| AgentAccessError::Unknown {
                        connection: connection.to_owned(),
                    })?;
            let mut record = registry
                .servers
                .get(connection)
                .map(|known| known.value.clone())
                .ok_or_else(|| AgentAccessError::Unknown {
                    connection: connection.to_owned(),
                })?;
            if record.request_removal().is_err() {
                return Ok(ServerTrust::Removed);
            }
            // The intent is stored first: a server that cannot be reached keeps it.
            registry.save_server(&store, record.clone()).await?;
            let Some(document) = registry.ca.as_ref().map(|ca| &ca.value) else {
                return Ok(ServerTrust::RemovalPending);
            };
            if self.remove_line(connection, document).await.is_err() {
                return Ok(ServerTrust::RemovalPending);
            }
            record.removal_confirmed((self.clock)());
            let state = state_of(record.state());
            registry.save_server(&store, record).await?;
            Ok(state)
        })
        .await
    }

    async fn remove_line(
        &self,
        connection: &str,
        document: &CaDocument,
    ) -> Result<(), AgentAccessError> {
        let host = self.host(connection)?;
        let text = host.file.read().await.map_err(host_error)?;
        let updated = without_lines(&text, document);
        if updated != text {
            host.file.replace(text, updated).await.map_err(host_error)?;
        }
        Ok(())
    }

    pub(super) async fn reconcile(
        &self,
        hub: &HubId,
        connection: &str,
    ) -> Result<ServerTrust, AgentAccessError> {
        let host = self.host(connection)?;
        let work = self.source.work(hub)?;
        let store = work.transient_store();
        Self::guarded(&work, async {
            let mut registry =
                Registry::load(&store, false)
                    .await?
                    .ok_or_else(|| AgentAccessError::Unknown {
                        connection: connection.to_owned(),
                    })?;
            let unknown = || AgentAccessError::Unknown {
                connection: connection.to_owned(),
            };
            let mut record = registry
                .servers
                .get(connection)
                .map(|known| known.value.clone())
                .ok_or_else(unknown)?;
            let document = registry
                .ca
                .as_ref()
                .map(|ca| &ca.value)
                .ok_or_else(unknown)?;
            let text = host.file.read().await.map_err(host_error)?;
            let action = record.reconcile(observe(&text, document), (self.clock)());
            if action == TrustAction::RemoveLine {
                let updated = without_lines(&text, document);
                if host.file.replace(text, updated).await.is_ok() {
                    record.removal_confirmed((self.clock)());
                }
            }
            let state = state_of(record.state());
            registry.save_server(&store, record).await?;
            Ok(state)
        })
        .await
    }

    pub(super) async fn reissue(&self, hub: &HubId) -> Result<AgentCa, AgentAccessError> {
        let device = self.source.device(hub).await?;
        let work = self.source.work(hub)?;
        let store = work.transient_store();
        Self::guarded(&work, async {
            let mut registry = Registry::load(&store, true)
                .await?
                .ok_or(AgentAccessError::Hub(
                    nocterm_hubs_model::HubError::NotFound,
                ))?;
            let now = (self.clock)();
            let old = registry.ca.as_ref().map(|ca| &ca.value);
            let replaced = match old {
                Some(old) => old.replaced(fresh_seed()?, now),
                None => CaDocument::new(fresh_seed()?, now),
            }
            .map_err(records)?;
            let summary = ca_summary(&replaced);
            let line = line_of(&replaced)?;
            let fingerprint = replaced.ca().fingerprint().to_owned();
            registry.save_ca(&store, replaced).await?;
            let targets: Vec<SshTrustRecord> = registry
                .servers
                .values()
                .filter(|server| server.value.state() == TrustState::Trusted)
                .map(|server| server.value.clone())
                .collect();
            for record in targets {
                let Ok(host) = self.host(&record.profile_id) else {
                    continue;
                };
                let Some(document) = registry.ca.as_ref().map(|ca| &ca.value) else {
                    continue;
                };
                let moved = async {
                    let text = host.file.read().await.map_err(host_error)?;
                    let updated = with_line(&without_lines(&text, document), &line);
                    host.file.replace(text, updated).await.map_err(host_error)
                }
                .await;
                if moved.is_ok()
                    && let Ok(fresh) = SshTrustRecord::installed(
                        record.profile_id.clone(),
                        record.host.clone(),
                        record.user.clone(),
                        record.host_key_fingerprint.clone(),
                        fingerprint.clone(),
                        now,
                        device,
                    )
                {
                    registry.save_server(&store, fresh).await?;
                }
            }
            Ok(summary)
        })
        .await
    }
}
