//! Preserve original SDK failures without ordinary formatting of their payloads.
use nocterm_cloud_model::engine::EngineError;
use nocterm_core::SharedCause;
use std::{error::Error, fmt};

pub(crate) fn provider_error(error: impl Error + Send + Sync + 'static) -> EngineError {
    let unsupported = (&error as &dyn Error)
        .downcast_ref::<rig_core::error::ProviderError>()
        .is_some_and(|error| matches!(error, rig_core::error::ProviderError::UnsupportedOption(_)));
    let cause = SharedCause::new(ProviderFailure(error));
    if unsupported {
        EngineError::UnsupportedCapability(cause)
    } else {
        EngineError::Provider(cause)
    }
}
#[derive(thiserror::Error)]
#[error("provider attempt failed")]
struct ProviderFailure<E: Error + Send + Sync + 'static>(#[source] E);
impl<E: Error + Send + Sync + 'static> fmt::Debug for ProviderFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderFailure")
    }
}
#[derive(Debug, thiserror::Error)]
#[error("provider continuation requires unsupported opaque or reasoning state")]
pub(crate) struct UnsupportedContinuation;
