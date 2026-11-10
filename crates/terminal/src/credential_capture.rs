//! One immutable original credential/source owner; reconnect never recaptures it.
use nocterm_hubs_model::source_identity::{LocalAccountPartition, LocalAccountSource};
use nocterm_session::{
    ConnectionOpenOptions, ConnectionSource, ConnectionSourceError, ConnectionSourceGuard,
    CredentialFuture, CredentialId, CredentialStore, CredentialStoreError, Secret, SecretRequest,
};
use nocterm_workspace::SessionSource;
use std::sync::Arc;
enum CredentialOrigin {
    ThisComputer,
    Account {
        partition: LocalAccountPartition,
        source: Arc<dyn LocalAccountSource>,
    },
    Inherited(SessionSource),
    Unavailable,
}
struct CredentialOriginGuard {
    origin: CredentialOrigin,
}
impl CredentialOriginGuard {
    fn partition(&self) -> Option<LocalAccountPartition> {
        match &self.origin {
            CredentialOrigin::Account { partition, .. } => Some(partition.clone()),
            CredentialOrigin::Inherited(source) => source.partition().cloned(),
            _ => None,
        }
    }
}
impl ConnectionSourceGuard for CredentialOriginGuard {
    fn check(&self) -> Result<(), ConnectionSourceError> {
        let current = match &self.origin {
            CredentialOrigin::ThisComputer => true,
            CredentialOrigin::Unavailable => false,
            CredentialOrigin::Account { partition, source } => {
                source.check_current().is_ok() && source.partition() == *partition
            }
            CredentialOrigin::Inherited(source) => source.check_current().is_ok(),
        };
        if current {
            Ok(())
        } else {
            Err(ConnectionSourceError::Revoked)
        }
    }
}
/// Supplied native store plus stable source capability. Constructors confer no authentication.
#[derive(Clone)]
pub struct CredentialCapture {
    store: Arc<dyn CredentialStore>,
    origin: Arc<CredentialOriginGuard>,
    options: ConnectionOpenOptions,
}
impl CredentialCapture {
    fn captured(store: Arc<dyn CredentialStore>, origin: CredentialOrigin) -> Self {
        let origin = Arc::new(CredentialOriginGuard { origin });
        let source = match &origin.origin {
            CredentialOrigin::ThisComputer => ConnectionSource::default(),
            CredentialOrigin::Inherited(source) => source.connection().clone(),
            _ => ConnectionSource::new(origin.clone()),
        };
        Self {
            store,
            origin,
            options: ConnectionOpenOptions::new(source),
        }
    }
    pub fn this_computer(store: Arc<dyn CredentialStore>) -> Self {
        Self::captured(store, CredentialOrigin::ThisComputer)
    }
    pub fn account(store: Arc<dyn CredentialStore>, source: Arc<dyn LocalAccountSource>) -> Self {
        Self::captured(
            store,
            CredentialOrigin::Account {
                partition: source.partition(),
                source,
            },
        )
    }
    pub(crate) fn inherited_program(source: SessionSource) -> Self {
        Self::captured(
            Arc::new(UnavailableCredentials {
                error: CredentialStoreError::NotStorable,
            }),
            CredentialOrigin::Inherited(source),
        )
    }
    pub(crate) fn failed(error: CredentialStoreError) -> Self {
        Self::captured(
            Arc::new(UnavailableCredentials { error }),
            CredentialOrigin::Unavailable,
        )
    }
    pub fn with_relay(mut self, relay: Arc<dyn nocterm_session::RelayDialer>) -> Self {
        self.options = self.options.with_relay(relay);
        self
    }
    pub(crate) fn connection_options(&self) -> ConnectionOpenOptions {
        self.options.clone()
    }
    pub(crate) fn session_source(&self) -> SessionSource {
        SessionSource::new(self.partition(), self.options.source().clone())
    }
    pub(crate) fn store(&self) -> &Arc<dyn CredentialStore> {
        &self.store
    }
    pub(crate) fn partition(&self) -> Option<LocalAccountPartition> {
        self.origin.partition()
    }
    pub(crate) fn current(&self) -> bool {
        self.origin.check().is_ok()
    }
}
struct UnavailableCredentials {
    error: CredentialStoreError,
}
impl CredentialStore for UnavailableCredentials {
    fn is_available(&self) -> bool {
        false
    }
    fn wait_available(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        Box::pin(std::future::pending())
    }
    fn get(&self, _: CredentialId, _: &SecretRequest) -> CredentialFuture<Secret> {
        let error = self.error.clone();
        Box::pin(async move { Err(error) })
    }
    fn save(&self, _: String, _: &SecretRequest, _: Secret) -> CredentialFuture<CredentialId> {
        let error = self.error.clone();
        Box::pin(async move { Err(error) })
    }
}
