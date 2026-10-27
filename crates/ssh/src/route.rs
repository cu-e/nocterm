//! The chain of SSH connections that leads to the destination.
//!
//! The first hop is reached through the configured proxy, or through a stream
//! handed over by a hub relay; every further host is
//! reached by asking the previous hop for a `direct-tcpip` channel and running
//! the next SSH handshake over it. A hop is verified and signed in before a
//! forward is opened through it. The hops live in a [`Route`], which closes
//! them from the destination backwards.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use nocterm_session::{
    ConnectRequest, ConnectStage, Event, RelayDialer, SessionDriver, SessionError, Target,
};
use russh::{Disconnect, client::Handle};
use tokio::time::timeout;

use crate::{
    SshConfig, auth,
    login::{self, Client, Login, Observed},
    proxy::{self, Stream},
};

/// How long closing one hop may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// One established jump host.
struct Hop {
    handle: Handle<Client>,
    observed: Arc<Mutex<Observed>>,
    target: Target,
}

/// The established jump hosts, nearest first.
///
/// Dropping a route closes its hops in the background, so a route abandoned by
/// cancellation or an early return never leaves a connection behind.
pub(crate) struct Route {
    hops: Vec<Hop>,
    /// Opens the first stream when the route is relayed by a hub.
    relay: Option<Arc<dyn RelayDialer>>,
}

impl Route {
    fn new(relay: Option<Arc<dyn RelayDialer>>) -> Self {
        Self {
            hops: Vec::new(),
            relay,
        }
    }

    /// Disconnects the hops from the destination backwards.
    pub(crate) async fn close(mut self) {
        disconnect_all(std::mem::take(&mut self.hops)).await;
    }

    /// Blames the hop whose connection broke, when the destination was merely
    /// told that its transport went away.
    pub(crate) fn attribute(&self, error: SessionError) -> SessionError {
        if !matches!(error, SessionError::ConnectionLost(_)) {
            return error;
        }
        for (index, hop) in self.hops.iter().enumerate() {
            if let Some(message) = login::take_disconnect(&hop.observed) {
                return at_hop(index, &hop.target, SessionError::ConnectionLost(message));
            }
        }
        error
    }
}

impl Drop for Route {
    fn drop(&mut self) {
        let hops = std::mem::take(&mut self.hops);
        if hops.is_empty() {
            return;
        }
        // Never block here; without a runtime the connections end with the process.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(disconnect_all(hops));
        }
    }
}

async fn disconnect_all(hops: Vec<Hop>) {
    for hop in hops.into_iter().rev() {
        let goodbye = hop.handle.disconnect(Disconnect::ByApplication, "", "en");
        // Fails only if the connection is already gone.
        let _ = timeout(CLOSE_TIMEOUT, goodbye).await;
    }
}

fn at_hop(index: usize, target: &Target, source: SessionError) -> SessionError {
    SessionError::JumpHost {
        hop: index,
        host: target.to_string(),
        source: Box::new(source),
    }
}

/// Which side of a step went wrong.
enum Fault {
    /// The previous hop could not open the forward.
    Forward(SessionError),
    /// This step's own host failed.
    Own(SessionError),
}

/// Reaches the destination through every jump host, signing in on each.
///
/// Returns the destination's connection and the route carrying the hops.
pub(crate) async fn establish(
    config: &SshConfig,
    request: &ConnectRequest,
    relay: Option<Arc<dyn RelayDialer>>,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<(Handle<Client>, Route), SessionError> {
    let mut route = Route::new(relay);
    match extend(&mut route, config, request, driver, observed).await {
        Ok(handle) => Ok((handle, route)),
        Err(error) => {
            route.close().await;
            Err(error)
        }
    }
}

async fn extend(
    route: &mut Route,
    config: &SshConfig,
    request: &ConnectRequest,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, SessionError> {
    for (index, jump) in request.jumps.iter().enumerate() {
        let login = Login::jump(index, jump);
        let hop_observed = Arc::new(Mutex::new(Observed::default()));
        let handle = link(route, &login, config, request, driver, &hop_observed).await?;
        route.hops.push(Hop {
            handle,
            observed: hop_observed,
            target: jump.target.clone(),
        });
    }
    let login = Login::destination(request);
    link(route, &login, config, request, driver, observed).await
}

/// One step: reach the host, verify its key, sign in.
async fn link(
    route: &Route,
    login: &Login<'_>,
    config: &SshConfig,
    request: &ConnectRequest,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, SessionError> {
    let own = |error| match login.hop() {
        Some(hop) => at_hop(hop.index, &hop.target, error),
        None => error,
    };
    let host_keys = login::load_host_keys(config).await.map_err(own)?;
    driver
        .emit(Event::Connecting(ConnectStage::Connecting {
            hop: login.hop().map(|hop| hop.index),
        }))
        .await;
    let mut handle = reach(route, login, &host_keys, request, observed)
        .await
        .map_err(|fault| match fault {
            Fault::Own(error) => own(error),
            Fault::Forward(error) => forward_fault(route, error),
        })?;
    login::verify_host(&handle, login, &host_keys, driver, observed)
        .await
        .map_err(own)?;

    driver
        .emit(Event::Connecting(ConnectStage::Authenticating {
            hop: login.hop().map(|hop| hop.index),
        }))
        .await;
    auth::authenticate(&mut handle, login, config, driver)
        .await
        .map_err(own)?;
    Ok(handle)
}

/// Opens the stream to the host and runs the handshake over it.
async fn reach(
    route: &Route,
    login: &Login<'_>,
    host_keys: &crate::host_keys::HostKeys,
    request: &ConnectRequest,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, Fault> {
    let target = login.target();
    let attempt = async {
        let stream = open_stream(route, target, request).await?;
        login::handshake(
            login,
            stream,
            host_keys,
            request.keepalive_interval,
            observed,
        )
        .await
        .map_err(Fault::Own)
    };
    timeout(request.connect_timeout, attempt)
        .await
        .map_err(|_| {
            Fault::Own(SessionError::TimedOut {
                host: target.host.clone(),
                seconds: request.connect_timeout.as_secs(),
            })
        })?
}

async fn open_stream(
    route: &Route,
    target: &Target,
    request: &ConnectRequest,
) -> Result<Box<dyn Stream>, Fault> {
    let unreachable = |reason: String| SessionError::Unreachable {
        host: target.host.clone(),
        reason,
    };
    let Some(previous) = route.hops.last() else {
        return proxy::connect(target, &request.proxy, route.relay.as_ref())
            .await
            .map_err(|error| Fault::Own(error.into_session(&target.host)));
    };
    proxy::validate_host(&target.host)
        .map_err(|error| Fault::Forward(unreachable(error.to_string())))?;
    let channel = previous
        .handle
        .channel_open_direct_tcpip(target.host.clone(), target.port.into(), "127.0.0.1", 0)
        .await
        .map_err(|error| Fault::Forward(unreachable(error.to_string())))?;
    Ok(Box::new(channel.into_stream()))
}

/// A refused forward is the previous hop's failure to reach this host.
fn forward_fault(route: &Route, error: SessionError) -> SessionError {
    match route.hops.last() {
        Some(previous) => at_hop(route.hops.len() - 1, &previous.target, error),
        None => error,
    }
}
