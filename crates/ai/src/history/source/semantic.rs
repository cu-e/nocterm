//! Certified native semantic data stays inseparable from its pending projection.
use super::SourceContext;
use crate::{
    history::MeasurementError,
    model::{
        json::deferred::JsonDecodeError,
        native::NativeOwned,
        notice::{SessionNotice, SessionUpdate},
        retention::document::PolicyLiability,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum SourceSemanticError {
    #[error("semantic payload belongs to another native source")]
    ForeignSource,
    #[error(transparent)]
    Allocation(#[from] JsonDecodeError),
    #[error(transparent)]
    Measurement(#[from] MeasurementError),
}
/// Refusal preserves the original native input; dropping it frees that payload
/// and its authentic physical receipts without publishing any new live state.
pub struct SourceSemanticRefusal<T = SessionNotice> {
    input: NativeOwned<T>,
    cause: SourceSemanticError,
}
impl SourceSemanticRefusal<SessionNotice> {
    pub fn notice(&self) -> &SessionNotice {
        self.input.get()
    }
}
impl<T> SourceSemanticRefusal<T> {
    pub fn cause(&self) -> &SourceSemanticError {
        &self.cause
    }
}
impl<T> std::fmt::Debug for SourceSemanticRefusal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SourceSemanticRefusal")
            .field(&self.cause)
            .finish()
    }
}
impl<T> std::fmt::Display for SourceSemanticRefusal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.cause.fmt(f)
    }
}
impl<T> std::error::Error for SourceSemanticRefusal<T> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
/// Drop order is payload, semantic promise, original context. This owner never
/// owns or remints the connection's unique Source lease.
pub struct ManagedNotice {
    payload: Option<NativeOwned<SessionNotice>>,
    liability: PolicyLiability,
    source: SourceContext,
}
impl ManagedNotice {
    pub fn notice(&self) -> &SessionNotice {
        self.payload
            .as_ref()
            .expect("unconsumed semantic notice")
            .get()
    }
    pub(crate) fn belongs_to(
        &self,
        scope: &crate::model::retention::document::DocumentRetention,
    ) -> bool {
        self.source.0.scope.same_document(scope)
    }
    pub(crate) fn take_update(&mut self) -> (NativeOwned<SessionUpdate>, &mut PolicyLiability) {
        let (notice, allocation) = self
            .payload
            .take()
            .expect("unconsumed semantic notice")
            .into_parts();
        let SessionNotice { session, update } = notice;
        drop(session);
        (
            NativeOwned::from_parts(update, allocation),
            &mut self.liability,
        )
    }
}
impl ManagedNotice {
    #[expect(
        clippy::result_large_err,
        reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
    )]
    pub(super) fn certify(
        source: &SourceContext,
        input: NativeOwned<SessionNotice>,
    ) -> Result<ManagedNotice, SourceSemanticRefusal> {
        let result = (|| {
            if !input.belongs_to_ledger(source.ledger()) {
                return Err(SourceSemanticError::ForeignSource);
            }
            let policy = super::policy::SourcePolicy::notice(input.get())?.bytes();
            Ok(source.ledger().reserve_source_policy(policy)?)
        })();
        match result {
            Ok(liability) => Ok(ManagedNotice {
                payload: Some(input),
                liability,
                source: source.clone(),
            }),
            Err(cause) => Err(SourceSemanticRefusal { input, cause }),
        }
    }
}

#[cfg(test)]
mod tests;

macro_rules! control_owner {
    ($name:ident, $ty:ty, $borrow:ident, $policy:ident, installed) => {
        control_owner!($name, $ty, $borrow, $policy);
        impl $name {
            pub(crate) fn belongs_to(
                &self,
                scope: &crate::model::retention::document::DocumentRetention,
            ) -> bool {
                self._source.0.scope.same_document(scope)
            }
            pub(crate) fn into_parts(self) -> (NativeOwned<$ty>, PolicyLiability, SourceContext) {
                (self.payload, self._liability, self._source)
            }
        }
    };
    ($name:ident, $ty:ty, $borrow:ident, $policy:ident) => {
        /// One original native control result and its opaque semantic promise.
        pub struct $name {
            payload: NativeOwned<$ty>,
            _liability: PolicyLiability,
            _source: SourceContext,
        }
        impl $name {
            pub fn $borrow(&self) -> &$ty {
                self.payload.get()
            }
            #[expect(
                clippy::result_large_err,
                reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
            )]
            pub(super) fn certify(
                source: &SourceContext,
                input: NativeOwned<$ty>,
            ) -> Result<Self, SourceSemanticRefusal<$ty>> {
                let result = (|| {
                    if !input.belongs_to_ledger(source.ledger()) {
                        return Err(SourceSemanticError::ForeignSource);
                    }
                    let policy = super::policy::SourcePolicy::$policy(input.get())?.bytes();
                    Ok(source.ledger().reserve_source_policy(policy)?)
                })();
                match result {
                    Ok(liability) => Ok(Self {
                        payload: input,
                        _liability: liability,
                        _source: source.clone(),
                    }),
                    Err(cause) => Err(SourceSemanticRefusal { input, cause }),
                }
            }
        }
    };
}
control_owner!(
    ManagedOpened,
    crate::model::session::SessionOpened,
    opened,
    opened,
    installed
);
control_owner!(
    ManagedPermission,
    crate::model::auth::ProviderPermission,
    permission,
    permission
);
control_owner!(
    ManagedAgentInfo,
    crate::connection::AgentInfo,
    info,
    info,
    installed
);
