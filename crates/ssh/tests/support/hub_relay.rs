//! A hub relay stand-in: a `RelayDialer` that hands the transport a bounded relayed
//! stream, with the destination reached over local TCP, refused, or never answered.
use std::sync::Arc;

use futures::{FutureExt as _, future::BoxFuture};
use nocterm_session::{
    ConnectRequest, RELAY_FRAME_BYTES, RelayDialer, RelayError, RelayPeer, RelayStream, Target,
    testing::{RelayRequest, ScriptedRelay},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
    sync::Semaphore,
    time::{Duration, timeout},
};

pub(crate) const HUB: &str = "hub.test:7443";

/// A scripted relay whose opened streams are carried to the destination over local TCP.
///
/// Refusals and silence need no more than a [`ScriptedRelay`] itself.
#[derive(Clone)]
pub(crate) struct TcpRelay {
    script: ScriptedRelay,
    ended: Arc<Semaphore>,
}

impl TcpRelay {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            script: ScriptedRelay::default(),
            ended: Arc::new(Semaphore::new(0)),
        })
    }

    pub(crate) fn asked(&self) -> Vec<RelayRequest> {
        self.script.asked()
    }

    /// Resolves once a relayed connection has ended in both directions.
    pub(crate) async fn connection_ended(&self) {
        timeout(Duration::from_secs(5), self.ended.acquire())
            .await
            .expect("the relayed connection did not end")
            .unwrap()
            .forget();
    }
}

impl RelayDialer for TcpRelay {
    fn capture(
        &self,
        _hub: &str,
        _node: Option<&str>,
        _target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        Ok(nocterm_session::testing::capture_relay(
            Arc::new(self.clone()),
            _hub,
            _node,
            _target,
        ))
    }

    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        let dial = self.script.dial(hub, node, target);
        let script = self.script.clone();
        let ended = self.ended.clone();
        let port = target.port;
        async move {
            let tcp = TcpStream::connect(("127.0.0.1", port))
                .await
                .map_err(|error| RelayError::Unreachable(nocterm_core::SharedCause::new(error)))?;
            let stream = dial.await?;
            tokio::spawn(carry(tcp, script.next_peer().await, ended));
            Ok(stream)
        }
        .boxed()
    }
}

/// Moves bytes between the destination and the stream until both directions end.
async fn carry(tcp: TcpStream, peer: RelayPeer, ended: Arc<Semaphore>) {
    let (mut read, mut write) = tcp.into_split();
    let (from_transport, to_transport) = peer.into_parts();
    let upload = async {
        while let Ok(frame) = from_transport.recv().await {
            if write.write_all(&frame).await.is_err() {
                break;
            }
        }
        let _ = write.shutdown().await;
    };
    let download = async {
        let mut buffer = vec![0; RELAY_FRAME_BYTES];
        loop {
            match read.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if to_transport.send(buffer[..count].to_vec()).await.is_err() {
                        break;
                    }
                }
            }
        }
        drop(to_transport);
    };
    tokio::join!(upload, download);
    ended.add_permits(1);
}

/// A request for `target` through the hub, naming the node `node`.
pub(crate) fn through_hub(mut request: ConnectRequest, node: Option<&str>) -> ConnectRequest {
    request.proxy = nocterm_session::ProxyConfig::Hub {
        address: HUB.into(),
        node: node.map(Into::into),
    };
    request
}
