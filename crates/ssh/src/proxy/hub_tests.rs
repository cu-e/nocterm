//! The hub route of [`connect`]: what is asked of the relay and how its answers surface.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures::{FutureExt as _, future::BoxFuture};
use nocterm_session::{RelayRefusal, RelayStream};
use tokio::time::timeout;

use super::*;

type Dialed = (String, Option<String>, Target);

enum Answer {
    Stream(RelayStream),
    Refuse(RelayError),
    Hang(Arc<AtomicBool>),
}

/// Answers the next dial with a scripted outcome and records what was asked.
struct Dialer {
    asked: Mutex<Vec<Dialed>>,
    answers: Mutex<Vec<Answer>>,
}

impl Dialer {
    fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::default(),
            answers: Mutex::new(vec![answer]),
        })
    }
}

/// Flags when the dial future is dropped before it finished.
struct Abandoned(Arc<AtomicBool>);
impl Drop for Abandoned {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl RelayDialer for Dialer {
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
        match self.answers.lock().unwrap().pop().unwrap() {
            Answer::Stream(stream) => async move { Ok(stream) }.boxed(),
            Answer::Refuse(error) => async move { Err(error) }.boxed(),
            Answer::Hang(flag) => async move {
                let _guard = Abandoned(flag);
                futures::future::pending().await
            }
            .boxed(),
        }
    }
}

fn target() -> Target {
    Target::new("user", "inner.example", 22)
}

fn hub_route(node: Option<&str>) -> ProxyConfig {
    ProxyConfig::Hub {
        address: "hub.example:7443".into(),
        node: node.map(Into::into),
    }
}

#[tokio::test]
async fn the_relay_is_asked_for_the_hub_node_and_target_and_its_stream_carries_bytes() {
    let (stream, peer) = RelayStream::pair();
    let dialer = Dialer::new(Answer::Stream(stream));
    let relay: Arc<dyn RelayDialer> = dialer.clone();
    let mut io = connect(&target(), &hub_route(Some("office")), Some(&relay))
        .await
        .unwrap();
    assert_eq!(
        dialer.asked.lock().unwrap().as_slice(),
        [(
            "hub.example:7443".to_owned(),
            Some("office".to_owned()),
            target()
        )]
    );
    let (from_transport, to_transport) = peer.into_parts();
    io.write_all(b"SSH-2.0-test\r\n").await.unwrap();
    assert_eq!(from_transport.recv().await.unwrap(), b"SSH-2.0-test\r\n");
    to_transport
        .send(b"SSH-2.0-host\r\n".to_vec())
        .await
        .unwrap();
    let mut banner = [0; 14];
    io.read_exact(&mut banner).await.unwrap();
    assert_eq!(&banner, b"SSH-2.0-host\r\n");
}

#[tokio::test]
async fn a_refusal_surfaces_as_a_relay_error_of_the_named_hub() {
    let refusal = RelayError::Refused {
        reason: RelayRefusal::PrivateNetwork,
    };
    let dialer = Dialer::new(Answer::Refuse(refusal.clone()));
    let relay: Arc<dyn RelayDialer> = dialer;
    let Err(error) = connect(&target(), &hub_route(None), Some(&relay)).await else {
        panic!("a refused dial must fail");
    };
    assert_eq!(
        error.into_session("inner.example"),
        SessionError::Relay {
            hub: "hub.example:7443".into(),
            source: refusal,
        }
    );
}

#[tokio::test]
async fn a_hub_route_without_a_relay_fails_to_connect() {
    let Err(error) = connect(&target(), &hub_route(None), None).await else {
        panic!("a hub route needs a relay");
    };
    assert!(matches!(error, ConnectError::NoRelay));
    assert!(matches!(
        error.into_session("inner.example"),
        SessionError::Unreachable { host, .. } if host == "inner.example"
    ));
}

#[tokio::test]
async fn abandoning_the_connection_abandons_the_dial() {
    let abandoned = Arc::new(AtomicBool::new(false));
    let relay: Arc<dyn RelayDialer> = Dialer::new(Answer::Hang(abandoned.clone()));
    let waited = timeout(
        Duration::from_millis(50),
        connect(&target(), &hub_route(None), Some(&relay)),
    )
    .await;
    assert!(waited.is_err(), "the dial never answers");
    assert!(abandoned.load(Ordering::SeqCst));
}

#[tokio::test]
async fn the_target_host_is_validated_before_the_relay_is_asked() {
    let dialer = Dialer::new(Answer::Refuse(RelayError::Closed));
    let relay: Arc<dyn RelayDialer> = dialer.clone();
    let hostile = Target::new("user", "host\r\nX: evil", 22);
    assert!(matches!(
        connect(&hostile, &hub_route(None), Some(&relay)).await,
        Err(ConnectError::Io(_))
    ));
    assert!(dialer.asked.lock().unwrap().is_empty());
}
