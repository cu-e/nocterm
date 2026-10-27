//! The hub route of [`connect`]: what is asked of the relay and how its answers surface.
use std::{sync::Arc, time::Duration};

use nocterm_session::RelayRefusal;
use nocterm_session::testing::{RelayReply, RelayRequest, ScriptedRelay};
use tokio::time::timeout;

use super::*;

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
    let hub = ScriptedRelay::default();
    let relay: Arc<dyn RelayDialer> = Arc::new(hub.clone());
    let mut io = connect(&target(), &hub_route(Some("office")), Some(&relay))
        .await
        .unwrap();
    assert_eq!(
        hub.asked(),
        [RelayRequest {
            hub: "hub.example:7443".into(),
            node: Some("office".into()),
            target: target(),
        }]
    );
    let (from_transport, to_transport) = hub.next_peer().await.into_parts();
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
    let relay: Arc<dyn RelayDialer> = Arc::new(ScriptedRelay::refusing(refusal.clone()));
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
    let hub = ScriptedRelay::default();
    hub.reply(RelayReply::Hang);
    let relay: Arc<dyn RelayDialer> = Arc::new(hub.clone());
    let waited = timeout(
        Duration::from_millis(50),
        connect(&target(), &hub_route(None), Some(&relay)),
    )
    .await;
    assert!(waited.is_err(), "the dial never answers");
    assert_eq!(hub.abandoned_dials(), 1);
}

#[tokio::test]
async fn the_target_host_is_validated_before_the_relay_is_asked() {
    let hub = ScriptedRelay::refusing(RelayError::Closed);
    let relay: Arc<dyn RelayDialer> = Arc::new(hub.clone());
    let hostile = Target::new("user", "host\r\nX: evil", 22);
    assert!(matches!(
        connect(&hostile, &hub_route(None), Some(&relay)).await,
        Err(ConnectError::Io(_))
    ));
    assert!(hub.asked().is_empty());
}
