#[derive(Clone, Debug, thiserror::Error)]
pub enum ModelError {
    #[error("retained owner transition failed: {0}")]
    Retention(#[source] nocterm_core::SharedError<super::retention::RetentionError>),
    #[error("JSON allocation arithmetic overflow")]
    JsonArithmetic,
    #[error("JSON allocation reservation failed: {0}")]
    JsonAllocation(#[source] nocterm_core::SharedError<std::collections::TryReserveError>),
    #[error("Unsupported semantic record: {0}")]
    Unsupported(&'static str),
    #[error("Extension data exceeds the bounded document capacity")]
    ExtensionLimit,
    #[error("Reserved client metadata cannot be accepted as provider evidence")]
    ReservedMetadata,
}

/// Bounded remote semantics. SDK/native transport errors stay in the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteFaultKind {
    Rpc,
    AuthenticationRequired,
    RestoreUnavailable,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct RemoteFault {
    code: i32,
    message: String,
    kind: RemoteFaultKind,
}
impl RemoteFault {
    /// The adapter must redact remote text before crossing this boundary.
    /// A remote report is a semantic leaf, with no retained SDK source/downcast path.
    pub fn new(code: i32, message: String) -> Self {
        Self {
            code,
            message: message.chars().take(4096).collect(),
            kind: RemoteFaultKind::Rpc,
        }
    }
    /// Move the adapter's already redacted and bounded original text without
    /// copying its backing or replacing its construction authority.
    pub(crate) fn from_native(
        code: i32,
        kind: RemoteFaultKind,
        message: super::native::NativeOwned<String>,
    ) -> Result<super::native::NativeOwned<Self>, super::json::deferred::JsonDecodeError> {
        if message.as_str().chars().take(4097).count() > 4096 {
            return Err(super::json::deferred::JsonDecodeError::RemoteTextLimit { limit: 4096 });
        }
        let (message, allocation) = message.into_parts();
        Ok(super::native::NativeOwned::from_parts(
            Self {
                code,
                message,
                kind,
            },
            allocation,
        ))
    }
    pub fn kind(&self) -> RemoteFaultKind {
        self.kind
    }
    pub fn code(&self) -> i32 {
        self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        json::deferred::{DecodeLedger, JsonDecodeError, literal::LiteralBuilder},
        native::NativeOwned,
        retention::RetentionAuthority,
    };

    fn native_text(ledger: &DecodeLedger, value: &str) -> NativeOwned<String> {
        let mut builder = LiteralBuilder::new(ledger).unwrap();
        let mut text = builder.exact_text(value.len()).unwrap();
        text.append(value).unwrap();
        builder.string(text.finish().unwrap()).unwrap();
        NativeOwned::text(builder.finish_native().unwrap().into_text().unwrap())
    }

    #[test]
    fn remote_fault_has_bounded_semantic_text_without_transport_source() {
        let fault = RemoteFault::new(-32000, "界".repeat(4097));
        assert_eq!(fault.code(), -32000);
        assert_eq!(fault.kind(), RemoteFaultKind::Rpc);
        assert_eq!(fault.message().chars().count(), 4096);
        assert!(std::error::Error::source(&fault).is_none());
    }

    #[test]
    fn native_remote_fault_moves_original_text_and_preserves_semantic_kind() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = ledger.live_requested_bytes();
        for kind in [
            RemoteFaultKind::Rpc,
            RemoteFaultKind::AuthenticationRequired,
            RemoteFaultKind::RestoreUnavailable,
        ] {
            let text = native_text(&ledger, "界 message");
            let pointer = text.as_str().as_ptr();
            let retained = ledger.live_requested_bytes();
            let fault = crate::model::native::remote::remote_fault(-32000, kind, text).unwrap();
            assert_eq!(fault.get().message().as_ptr(), pointer);
            assert_eq!(fault.get().message(), "界 message");
            assert_eq!(fault.get().kind(), kind);
            assert_eq!(ledger.live_requested_bytes(), retained);
            assert!(std::error::Error::source(fault.get()).is_none());
            let error = crate::history::source::ManagedCommandError::Remote(fault);
            assert_eq!(
                error.authentication_required(),
                kind == RemoteFaultKind::AuthenticationRequired
            );
            assert_eq!(
                error.restore_unavailable(),
                kind == RemoteFaultKind::RestoreUnavailable
            );
            let prefix = if kind == RemoteFaultKind::AuthenticationRequired {
                "Authentication required: "
            } else {
                "Agent error: "
            };
            assert_eq!(error.to_string(), format!("{prefix}界 message"));
            drop(error);
            assert_eq!(ledger.live_requested_bytes(), baseline);
        }
    }

    #[test]
    fn native_remote_fault_accepts_scalar_boundary_and_rejects_oversized_original() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = ledger.live_requested_bytes();
        let text = "界".repeat(4096);
        let accepted = crate::model::native::remote::remote_fault(
            1,
            RemoteFaultKind::Rpc,
            native_text(&ledger, &text),
        )
        .unwrap();
        assert_eq!(accepted.get().message(), text);
        drop(accepted);
        assert_eq!(ledger.live_requested_bytes(), baseline);
        let text = "界".repeat(4097);
        let rejected = crate::model::native::remote::remote_fault(
            1,
            RemoteFaultKind::Rpc,
            native_text(&ledger, &text),
        );
        assert!(matches!(
            rejected,
            Err(JsonDecodeError::RemoteTextLimit { limit: 4096 })
        ));
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
    #[test]
    fn unsupported_local_restore_retains_original_static_diagnostics() {
        use crate::history::source::ManagedCommandError;
        for (fork, expected) in [
            (false, "The agent cannot reopen earlier chats."),
            (true, "The agent cannot fork chats."),
        ] {
            let error = ManagedCommandError::LocalRestoreUnsupported { fork };
            assert_eq!(error.to_string(), expected);
            assert!(error.restore_unavailable());
            assert!(!error.authentication_required());
            assert!(std::error::Error::source(&error).is_none());
        }
    }
}
