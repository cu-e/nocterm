//! Remote diagnostics move their already bounded original native text once.
use super::NativeOwned;
use crate::model::{
    diagnostic::{RemoteFault, RemoteFaultKind},
    json::deferred::JsonDecodeError,
};
pub fn remote_fault(
    code: i32,
    kind: RemoteFaultKind,
    message: NativeOwned<String>,
) -> Result<NativeOwned<RemoteFault>, JsonDecodeError> {
    RemoteFault::from_native(code, kind, message)
}
