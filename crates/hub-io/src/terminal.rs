//! The terminal of a hub session as a [`Transport`] of `nocterm-session`.
//!
//! A transport is bound to one hub session. Opening it attaches to the session, creating it
//! first for a new one. The hub contract keeps the attachment alive across dropped connections
//! and reports its progress; [`bridge`] shows that as session events and carries typing back.
//! Closing the session detaches: the process keeps running on its node.

mod bridge;

use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt as _;
use futures::channel::oneshot;
use nocterm_hub_client::HubClient;
use nocterm_hub_client::terminal::{TerminalOptions, run_terminal};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{HubError, HubId, HubSessionId, HubTerminalTarget};
use nocterm_session::{
    CloseReason, ConnectRequest, ConnectStage, Event, PtySize, Session, SessionDriver,
    SessionError, Transport,
};
use tokio::runtime::Handle;

use crate::pool::Pool;

pub(crate) use bridge::bridge;

/// Attaches terminal tabs to one session of one hub.
pub(crate) struct HubTerminalTransport {
    handle: Handle,
    pool: Arc<Pool>,
    hub: HubId,
    // A new session is created once; opening the transport again attaches to what was made.
    target: Arc<Mutex<HubTerminalTarget>>,
    // Told the id of a new session when the hub has made it.
    created: Arc<Mutex<Option<oneshot::Sender<HubSessionId>>>>,
}

impl HubTerminalTransport {
    /// The transport, and for a new session the receiver of its id.
    pub(crate) fn new(
        handle: Handle,
        pool: Arc<Pool>,
        hub: HubId,
        target: HubTerminalTarget,
    ) -> (Self, Option<oneshot::Receiver<HubSessionId>>) {
        let (made, created) = match target {
            HubTerminalTarget::New { .. } => {
                let (made, created) = oneshot::channel();
                (Some(made), Some(created))
            }
            HubTerminalTarget::Existing(_) => (None, None),
        };
        let transport = Self {
            handle,
            pool,
            hub,
            target: Arc::new(Mutex::new(target)),
            created: Arc::new(Mutex::new(made)),
        };
        (transport, created)
    }
}

impl Transport for HubTerminalTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        let (session, driver) = nocterm_session::channel(None);
        let attachment = Attachment {
            pool: Arc::clone(&self.pool),
            hub: self.hub.clone(),
            target: Arc::clone(&self.target),
            created: Arc::clone(&self.created),
            size: request.size,
        };
        self.handle.spawn(async move {
            let _ = driver
                .emit(Event::Connecting(ConnectStage::Connecting { hop: None }))
                .await;
            let reason = attachment.run(&driver).await;
            let _ = driver.emit(Event::Closed(reason)).await;
        });
        session
    }
}

struct Attachment {
    pool: Arc<Pool>,
    hub: HubId,
    target: Arc<Mutex<HubTerminalTarget>>,
    created: Arc<Mutex<Option<oneshot::Sender<HubSessionId>>>>,
    size: PtySize,
}

impl Attachment {
    /// Attaches and shows the terminal until it ends; says why it ended.
    async fn run(&self, driver: &SessionDriver) -> CloseReason {
        let client = tokio::select! {
            client = self.pool.fresh(&self.hub) => client,
            () = driver.closed() => return CloseReason::ClosedByUser,
        };
        let client = match client {
            Ok(client) => client,
            Err(error) => return bridge::failed(&error),
        };
        // Creating a session is not abandoned half way: it either exists or not, and a tab
        // closed meanwhile leaves it running on its node, listed in the sessions panel.
        let session_id = match self.session_for(&client).await {
            Ok(session_id) => session_id,
            Err(error) => return bridge::failed(&error),
        };
        if driver.closed().now_or_never().is_some() {
            return CloseReason::ClosedByUser;
        }
        let options = TerminalOptions {
            session_id: session_id.as_str().to_owned(),
            cols: self.size.cols,
            rows: self.size.rows,
            write: true,
        };
        let end = bridge(driver, |input, events| {
            run_terminal(&client, options, input, events)
        })
        .await;
        bridge::close_reason(end.map_err(|error| self.pool.failed(&self.hub, error)))
    }

    /// The session to attach to, which a new one creates first.
    async fn session_for(&self, client: &HubClient) -> Result<HubSessionId, HubError> {
        let target = self
            .target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let session = match target {
            HubTerminalTarget::Existing(session) => session,
            HubTerminalTarget::New { org, node, title } => {
                let request = v1::CreateSessionRequest {
                    org_id: org.as_str().to_owned(),
                    node_id: node
                        .map(|node| node.as_str().to_owned())
                        .unwrap_or_default(),
                    spec: Some(v1::SessionSpec {
                        kind: Some(v1::session_spec::Kind::Shell(v1::ShellSpec::default())),
                        title,
                        ..v1::SessionSpec::default()
                    }),
                    cols: self.size.cols.into(),
                    rows: self.size.rows.into(),
                };
                let created = client
                    .sessions()
                    .create_session(request)
                    .await
                    .map_err(|status| {
                        self.pool
                            .failed(&self.hub, nocterm_hub_client::ClientError::from(status))
                    })?
                    .into_inner();
                let session = HubSessionId::new(created.session_id);
                *self.target.lock().unwrap_or_else(PoisonError::into_inner) =
                    HubTerminalTarget::Existing(session.clone());
                let made = self
                    .created
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(made) = made {
                    // The tab may be gone already; the session still exists.
                    let _ = made.send(session.clone());
                }
                session
            }
        };
        Ok(session)
    }
}

/// A failure shown as the reason a session ended.
fn session_error(error: &HubError) -> SessionError {
    match error {
        HubError::Unreachable { .. } => SessionError::ConnectionLost(error.to_string()),
        other => SessionError::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests;
