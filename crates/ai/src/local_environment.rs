//! Device-local chat partitions, separate from General/Project membership.
use crate::history::ContextId;
use nocterm_hubs_model::source_identity::{
    LocalAccountPartition, LocalAccountSource, LocalAccountSourceError,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Descriptive persistence owner. This does not grant native or Cloud authority.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "account",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum LocalEnvironment {
    #[default]
    ThisComputer,
    HubAccount(LocalAccountPartition),
}
impl LocalEnvironment {
    /// Versioned local filename key only; the authoritative header is checked too.
    /// Display addresses are used solely by the explicit legacy configured variant.
    pub fn storage_key(&self) -> String {
        use sha2::{Digest as _, Sha256};
        let mut hash = Sha256::new();
        let mut part = |value: &[u8]| {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value);
        };
        part(b"nocterm/local-history-partition/v3");
        match self {
            Self::ThisComputer => part(b"this-computer"),
            Self::HubAccount(LocalAccountPartition::Stable(scope)) => {
                part(b"stable");
                part(scope.hub().instance().as_str().as_bytes());
                part(scope.hub().authority().as_bytes());
                part(scope.account().id().as_str().as_bytes());
                part(scope.account().key().as_bytes());
            }
            Self::HubAccount(LocalAccountPartition::LegacyConfigured(account)) => {
                part(b"legacy-configured");
                part(account.hub().to_string().as_bytes());
                part(account.account().as_str().as_bytes());
                part(account.key().as_bytes());
            }
        }
        let mut output = String::with_capacity(64);
        for byte in hash.finalize() {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
        }
        output
    }
    pub fn is_this_computer(&self) -> bool {
        matches!(self, Self::ThisComputer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatScope {
    environment: LocalEnvironment,
    context: ContextId,
}
impl ChatScope {
    pub fn new(environment: LocalEnvironment, context: ContextId) -> Self {
        Self {
            environment,
            context,
        }
    }
    pub fn this_computer(context: ContextId) -> Self {
        Self::new(LocalEnvironment::ThisComputer, context)
    }
    pub fn environment(&self) -> &LocalEnvironment {
        &self.environment
    }
    pub fn context(&self) -> ContextId {
        self.context
    }
}
impl From<ContextId> for ChatScope {
    fn from(context: ContextId) -> Self {
        Self::this_computer(context)
    }
}

/// A retained source capture. The original lease never becomes valid again
/// merely because a later sign-in selects the same persistent partition.
#[derive(Clone, Default)]
pub struct LocalEnvironmentAccess {
    source: Option<Arc<dyn LocalAccountSource>>,
    environment: Arc<LocalEnvironment>,
}
impl LocalEnvironmentAccess {
    pub fn this_computer() -> Self {
        Self::default()
    }
    pub fn capture(source: Arc<dyn LocalAccountSource>) -> Result<Self, LocalAccountSourceError> {
        source.check_current()?;
        Ok(Self {
            environment: Arc::new(LocalEnvironment::HubAccount(source.partition())),
            source: Some(source),
        })
    }
    pub fn environment(&self) -> &LocalEnvironment {
        &self.environment
    }
    pub fn check_current(&self) -> Result<(), LocalAccountSourceError> {
        if let Some(source) = &self.source {
            source.check_current()?;
            if *self.environment != LocalEnvironment::HubAccount(source.partition()) {
                return Err(LocalAccountSourceError::Stale);
            }
        }
        Ok(())
    }
}
