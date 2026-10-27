//! A hub relay stand-in: a `RelayDialer` that hands the transport a bounded relayed
//! stream, with the destination reached over local TCP, refused, or never answered.
use std::sync::{Arc, Mutex};

use futures::{FutureExt as _, future::BoxFuture};
use nocterm_session::{
    ConnectRequest, RELAY_FRAME_BYTES, RelayDialer, RelayError, RelayPeer, RelayStream, Target,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
    sync::{Semaphore, oneshot},
    time::{Duration, timeout},
};

/// What one dial asked for: the hub, its node and the destination.
pub(crate) type Dialed = (String, Option<String>, Target);

pub(crate) const HUB: &str = "hub.test:7443";

enum Mode {
    /// Connect to the destination on this machine and carry bytes both ways.
    Local,
    Refuse(RelayError),
    /// Open the stream and give the relay's end to the test, which never answers.
    Silent(Mutex<Option<oneshot::Sender<RelayPeer>>>),
}

pub(crate) struct FakeHub {
    mode: Mode,
    asked: Mutex<Vec<Dialed>>,
    ended: Arc<Semaphore>,
}

impl FakeHub {
    fn with(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            asked: Mutex::default(),
            ended: Arc::new(Semaphore::new(0)),
        })
    }

    pub(crate) fn local() -> Arc<Self> {
        Self::with(Mode::Local)
    }

    pub(crate) fn refusing(error: RelayError) -> Arc<Self> {
        Self::with(Mode::Refuse(error))
    }

    pub(crate) fn silent() -> (Arc<Self>, oneshot::Receiver<RelayPeer>) {
        let (give, take) = oneshot::channel();
        (Self::with(Mode::Silent(Mutex::new(Some(give)))), take)
    }

    pub(crate) fn asked(&self) -> Vec<Dialed> {
        self.asked.lock().unwrap().clone()
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

impl RelayDialer for FakeHub {
    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        self.asked
            .lock()
            .unwrap()
            .push((hub.to_owned(), node.map(str::to_owned), target.clone()));
        let (stream, peer) = RelayStream::pair();
        match &self.mode {
            Mode::Refuse(error) => {
                let error = error.clone();
                async move { Err(error) }.boxed()
            }
            Mode::Silent(give) => {
                let _ = give.lock().unwrap().take().map(|give| give.send(peer));
                async move { Ok(stream) }.boxed()
            }
            Mode::Local => {
                let port = target.port;
                let ended = self.ended.clone();
                async move {
                    let tcp = TcpStream::connect(("127.0.0.1", port))
                        .await
                        .map_err(|error| {
                            RelayError::Unreachable(nocterm_core::SharedCause::new(error))
                        })?;
                    tokio::spawn(carry(tcp, peer, ended));
                    Ok(stream)
                }
                .boxed()
            }
        }
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
