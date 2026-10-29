//! Immutable document and provider-resume facts captured before asynchronous work.
//! Owns: scoped chat keys and descriptive execution origins. No native I/O,
//! execution capability, provider preparation, views or current-selection lookup.
//! Ports provided: typed immutable scope validation. Settings/globals: none.
//! Invariant: conversation membership never grants workspace tool authority.
mod error;
use crate::{
    history::ContextId,
    local_environment::{ChatScope, LocalEnvironment},
};
pub use error::ExecutionOriginError;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Descriptive document identity. Storage validates the ID at its boundary;
/// possessing a key never grants access to a native resource or process.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatKey {
    scope: ContextId,
    environment: Arc<LocalEnvironment>,
    id: String,
}
impl ChatKey {
    pub fn new(scope: ContextId, id: String) -> Self {
        Self {
            scope,
            id,
            environment: Arc::new(LocalEnvironment::ThisComputer),
        }
    }
    pub fn from_header(header: &crate::history::SavedHeader) -> Self {
        Self::in_environment(header.chat_scope(), header.id.clone())
    }
    pub fn from_summary(summary: &crate::history::ChatSummary) -> Self {
        Self::in_environment(summary.chat_scope(), summary.id.clone())
    }
    pub fn in_environment(scope: ChatScope, id: String) -> Self {
        Self {
            scope: scope.context(),
            environment: Arc::new(scope.environment().clone()),
            id,
        }
    }
    pub fn environment(&self) -> &LocalEnvironment {
        &self.environment
    }
    pub fn chat_scope(&self) -> ChatScope {
        ChatScope::new(self.environment().clone(), self.scope)
    }
    pub fn scope(&self) -> ContextId {
        self.scope
    }
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Saved local provider cwd is a candidate for the injected WorkdirResolver.
/// It describes resume behavior and does not grant workspace tool access.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionOrigin {
    key: ChatKey,
    provider_resume: Option<PathBuf>,
}
impl ExecutionOrigin {
    pub fn new(key: ChatKey, provider_resume: Option<PathBuf>) -> Self {
        Self {
            key,
            provider_resume,
        }
    }
    pub fn key(&self) -> &ChatKey {
        &self.key
    }
    pub fn scope(&self) -> ContextId {
        self.key.scope()
    }
    pub fn provider_resume(&self) -> Option<&Path> {
        self.provider_resume.as_deref()
    }
    pub fn require_chat_scope(&self, expected: &ChatScope) -> Result<(), ExecutionOriginError> {
        if self.key.environment() != expected.environment() {
            return Err(ExecutionOriginError::EnvironmentMismatch);
        }
        self.require_scope(expected.context())
    }
    pub fn require_scope(&self, expected: ContextId) -> Result<(), ExecutionOriginError> {
        if self.scope() == expected {
            Ok(())
        } else {
            Err(ExecutionOriginError::ScopeMismatch {
                expected,
                actual: self.scope(),
            })
        }
    }
}
