//! Initialization certifies the same ordered result before publishing capabilities.
use super::{
    initialize, outgoing,
    rpc::{Cause, Rpc},
};
use nocterm_ai::{
    connection::ManagedStartupHandle,
    history::source::{ManagedAgentInfo, ManagedCommandError, ManagedSourceFuture},
    model::json::deferred::NativeAllocationError,
};
pub(super) fn initialize(
    rpc: &Rpc,
    startup: ManagedStartupHandle,
) -> Result<ManagedSourceFuture<Result<ManagedAgentInfo, ManagedCommandError>>, NativeAllocationError>
{
    let envelope = rpc.id().map_err(Cause::from).and_then(|id| {
        Ok(outgoing::initialize(
            rpc.budget(),
            &id,
            startup.facts().terminal_auth(),
        )?)
    });
    let context = rpc.budget().context().clone();
    let state = rpc.state().clone();
    rpc.operation_with_timeout(
        envelope,
        false,
        Some(std::time::Duration::from_secs(60)),
        move |value, _| {
            let initialized = initialize::initialized(value, startup.facts().launch().name())?;
            if initialized.version() != 1 {
                return Err(Cause::ProtocolVersion);
            }
            let restore = initialized.restore();
            let capabilities = &initialized.info().capabilities;
            let (fork, close) = (capabilities.fork, capabilities.close);
            let info = context.certify_info(initialized.into_info())?;
            state.initialize(restore, fork, close);
            Ok(info)
        },
    )
}
