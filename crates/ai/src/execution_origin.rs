//! Immutable document and provider-resume facts captured before asynchronous work.
//! Owns: scoped chat keys and descriptive execution origins. No native I/O,
//! execution capability, provider preparation, views or current-selection lookup.
//! Ports provided: typed immutable scope validation. Settings/globals: none.
//! Invariant: conversation membership never grants workspace tool authority.
mod error;
use crate::history::ContextId;
pub use error::ExecutionOriginError;
use std::path::{Path, PathBuf};

/// Descriptive document identity. Storage validates the ID at its boundary;
/// possessing a key never grants access to a native resource or process.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatKey {
    scope: ContextId,
    id: String,
}
impl ChatKey {
    pub fn new(scope: ContextId, id: String) -> Self {
        Self { scope, id }
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
