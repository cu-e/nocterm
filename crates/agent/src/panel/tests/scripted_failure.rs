//! Scripted causes are authored directly in the fixture's semantic vocabulary.
use nocterm_ai::{
    history::source::{ManagedCommandError, SourceContext},
    model::{
        diagnostic::RemoteFaultKind, json::deferred::NativeAllocationError, native::NativeCopies,
    },
};
#[derive(Clone, Copy)]
pub(super) enum RestoreFailure {
    Authentication(&'static str),
    Unavailable(&'static str),
    Io(&'static str),
    Rpc(i32, &'static str),
}
#[derive(Debug, thiserror::Error)]
#[error("I/O error: {0}")]
pub(super) struct FixtureIo(pub(super) &'static str);
impl RestoreFailure {
    pub(super) fn managed(
        self,
        context: &SourceContext,
    ) -> Result<ManagedCommandError, NativeAllocationError> {
        let native =
            context.prepare_error::<nocterm_ai::model::json::deferred::JsonDecodeError>(0)?;
        let (code, kind, text) = match self {
            Self::Io(text) => {
                return Ok(ManagedCommandError::Native(
                    context
                        .prepare_error::<FixtureIo>(0)?
                        .capture(FixtureIo(text)),
                ));
            }
            Self::Authentication(text) => (-32000, RemoteFaultKind::AuthenticationRequired, text),
            Self::Unavailable(text) => (-32601, RemoteFaultKind::RestoreUnavailable, text),
            Self::Rpc(code, text) => (code, RemoteFaultKind::Rpc, text),
        };
        Ok(
            match NativeCopies::new(context.ledger())
                .text(text)
                .and_then(|text| nocterm_ai::model::native::remote::remote_fault(code, kind, text))
            {
                Ok(fault) => ManagedCommandError::Remote(fault),
                Err(error) => ManagedCommandError::Native(native.capture(error)),
            },
        )
    }
}
