//! The operations on the registry: allow, deny, reconcile, reissue and overview.
//!
//! Every change that depends on a server first records the intent in the account's own
//! collection and only then edits the server, so a server that cannot be reached is never
//! forgotten: a denial stays `RemovalPending` and is finished on the next connection.
//!
//! A trusted server's line admits only that server's principal (`principal_for(ca, server)`,
//! line version 2). A server that still holds the shared principal of line version 1 needs a
//! CA update: its next connection rewrites the line and records the per-server form (trust
//! record schema 2, ADR 0147).

use std::future::Future;
use std::sync::Arc;

use nocterm_hub_model::cloud::ssh::SshServerId;
use nocterm_hubs_model::{
    AgentAccessError, AgentCa, AgentTrustOverview, HostChoice, HubId, ServerTrust, TrustedServer,
};
use nocterm_keys::agents::ssh_ca::{CaDocument, UserCa};
use nocterm_keys::agents::{
    CaLine, CaLineForm, Observation, ServerLine, SshTrustRecord, TrustAction, TrustState,
    find_ca_lines, remove_ca_lines, server_line,
};
use nocterm_ssh_keys::authorized_keys::{AgentHosts, HostAccess, HostAccessError, with_line};

use super::AccountWork;
use super::registry::{Registry, records};
use crate::account::operations::OpenedWork;

/// Milliseconds since the Unix epoch.
pub(super) type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

const LINE_COMMENT: &str = "nocterm agent access";

/// The line that lets the CA certify the server reached through `connection` only.
pub(super) fn line_of(document: &CaDocument, connection: &str) -> Result<String, AgentAccessError> {
    CaLine::for_server(
        document.ca().public_key(),
        &SshServerId::of_profile(connection),
        LINE_COMMENT,
    )
    .to_line()
    .map_err(records)
}

fn host_error(error: HostAccessError) -> AgentAccessError {
    match error {
        HostAccessError::NotConnected => AgentAccessError::NotConnected {
            connection: String::new(),
        },
        HostAccessError::Changed => AgentAccessError::HostChanged,
        HostAccessError::EditLocked { path } => AgentAccessError::HostEditLocked { path },
        HostAccessError::Unsupported(cause) => AgentAccessError::HostEditUnsupported { cause },
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
        needs_ca_update: record.state() == TrustState::Trusted
            && record.line() == CaLineForm::Shared,
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

fn fresh_ca() -> Result<UserCa, AgentAccessError> {
    UserCa::generate().map_err(records)
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

    pub(super) async fn server_choices(
        &self,
        hub: &HubId,
    ) -> Result<Vec<nocterm_cloud_conversation_model::ServerChoice>, AgentAccessError> {
        let overview = self.overview(hub).await?;
        Ok(overview
            .servers
            .into_iter()
            .filter(|server| server.state == ServerTrust::Trusted)
            .map(|server| nocterm_cloud_conversation_model::ServerChoice {
                server: nocterm_cloud_conversation_model::ServerKey::from_bytes(
                    *SshServerId::of_profile(&server.connection).as_bytes(),
                ),
                port: self.hosts.port(&server.connection),
                host: server.host,
                user: server.user,
                needs_ca_update: server.needs_ca_update,
                connection: server.connection,
            })
            .collect())
    }

    pub(super) async fn ca_line(
        &self,
        hub: &HubId,
        connection: &str,
    ) -> Result<String, AgentAccessError> {
        let work = self.source.work(hub)?;
        let store = work.transient_store();
        let registry = Self::guarded(&work, Registry::load(&store, false)).await?;
        let ca =
            registry
                .and_then(|registry| registry.ca)
                .ok_or_else(|| AgentAccessError::Unknown {
                    connection: String::new(),
                })?;
        line_of(&ca.value, connection)
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
                let document = CaDocument::new(fresh_ca()?, (self.clock)());
                registry.save_ca(&store, document).await?;
            }
            let document = &registry.ca.as_ref().map(|ca| &ca.value).ok_or_else(|| {
                AgentAccessError::Unknown {
                    connection: connection.to_owned(),
                }
            })?;
            let line = line_of(document, connection)?;
            let text = host.file.read().await.map_err(host_error)?;
            let updated = with_line(&without_lines(&text, document), &line);
            if updated != text {
                host.file.replace(text, updated).await.map_err(host_error)?;
            }
            let mut record = SshTrustRecord::installed(
                connection.to_owned(),
                host.identity.host.clone(),
                host.identity.user.clone(),
                host.identity.host_key_fingerprint.clone(),
                document.ca().fingerprint().to_owned(),
                (self.clock)(),
                device,
            )
            .map_err(records)?;
            record.set_line(CaLineForm::PerServer, (self.clock)());
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
            } else if record.state() == TrustState::Trusted {
                self.update_line(connection, document, &mut record, text, &host)
                    .await;
            }
            let state = state_of(record.state());
            registry.save_server(&store, record).await?;
            Ok(state)
        })
        .await
    }

    /// Gives a trusted server that still holds the shared principal its own line, and records
    /// the per-server form once the server has it. A failed edit keeps "needs a CA update"
    /// for the next connection.
    async fn update_line(
        &self,
        connection: &str,
        document: &CaDocument,
        record: &mut SshTrustRecord,
        text: String,
        host: &HostAccess,
    ) {
        let ca_key = document.ca().public_key();
        match server_line(&text, &ca_key, &SshServerId::of_profile(connection)) {
            ServerLine::PerServer if record.line() == CaLineForm::Shared => {
                record.set_line(CaLineForm::PerServer, (self.clock)());
            }
            ServerLine::Shared => {
                let Ok(line) = line_of(document, connection) else {
                    return;
                };
                let updated = with_line(&without_lines(&text, document), &line);
                if host.file.replace(text, updated).await.is_ok() {
                    record.set_line(CaLineForm::PerServer, (self.clock)());
                }
            }
            ServerLine::PerServer | ServerLine::Absent => {}
        }
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
                Some(old) => old.replaced(fresh_ca()?, now),
                None => CaDocument::new(fresh_ca()?, now),
            };
            let summary = ca_summary(&replaced);
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
                    let line = line_of(document, &record.profile_id)?;
                    let text = host.file.read().await.map_err(host_error)?;
                    let updated = with_line(&without_lines(&text, document), &line);
                    host.file.replace(text, updated).await.map_err(host_error)
                }
                .await;
                if moved.is_ok()
                    && let Ok(mut fresh) = SshTrustRecord::installed(
                        record.profile_id.clone(),
                        record.host.clone(),
                        record.user.clone(),
                        record.host_key_fingerprint.clone(),
                        fingerprint.clone(),
                        now,
                        device,
                    )
                {
                    fresh.set_line(CaLineForm::PerServer, now);
                    registry.save_server(&store, fresh).await?;
                }
            }
            Ok(summary)
        })
        .await
    }
}
