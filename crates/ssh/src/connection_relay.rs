//! One immutable relay requester, captured before the SSH worker is spawned.
use nocterm_session::{
    ConnectRequest, ConnectionOpenOptions, ConnectionSourceError, ProxyConfig, RelayDialer,
    SessionError,
};
use std::sync::Arc;
pub(crate) struct ConnectionRelay {
    port: Option<Arc<dyn RelayDialer>>,
}
impl ConnectionRelay {
    pub(crate) fn capture(
        request: &ConnectRequest,
        options: &ConnectionOpenOptions,
        desktop: Option<Arc<dyn RelayDialer>>,
    ) -> Result<Self, SessionError> {
        options.source().check()?;
        if matches!(request.proxy, ProxyConfig::Hub { .. })
            && options.source().is_restricted()
            && options.relay().is_none()
        {
            return Err(ConnectionSourceError::Unsupported.into());
        }
        let port = options.relay().cloned().or(desktop);
        let port = match (&request.proxy, port) {
            (ProxyConfig::Hub { address, node }, Some(port)) => {
                let target = request
                    .jumps
                    .first()
                    .map_or(&request.target, |jump| &jump.target);
                Some(
                    port.capture(address, node.as_deref(), target)
                        .map_err(|source| SessionError::Relay {
                            hub: address.clone(),
                            source,
                        })?,
                )
            }
            (_, port) => port,
        };
        Ok(Self { port })
    }
    pub(crate) fn into_port(self) -> Option<Arc<dyn RelayDialer>> {
        self.port
    }
}
