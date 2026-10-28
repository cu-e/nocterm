use crate::testing::{RelayReply, RelayRequest, ScriptedRelay};
use crate::{RelayDialer, RelayError, RelayRefusal, Target};
use futures::{FutureExt as _, executor::block_on};

fn target() -> Target {
    Target::new("user", "inner.example", 22)
}

#[test]
fn an_opened_stream_carries_bytes_to_the_peer_the_test_receives() {
    let relay = ScriptedRelay::default();
    block_on(async {
        let stream = relay.dial("hub", Some("office"), &target()).await.unwrap();
        let (to_relay, from_relay) = stream.into_parts();
        let (from_transport, to_transport) = relay.next_peer().await.into_parts();
        to_relay.send(b"up".to_vec()).await.unwrap();
        assert_eq!(from_transport.recv().await.unwrap(), b"up");
        to_transport.send(b"down".to_vec()).await.unwrap();
        assert_eq!(from_relay.recv().await.unwrap(), b"down");
    });
}

#[test]
fn every_dial_is_recorded_in_order_with_its_hub_node_and_target() {
    let relay = ScriptedRelay::default();
    drop(relay.dial("a", Some("n"), &target()));
    drop(relay.dial("b", None, &target()));
    let hubs: Vec<_> = relay.asked().into_iter().map(|asked| asked.hub).collect();
    assert_eq!(hubs, ["a", "b"]);
    assert_eq!(
        relay.asked()[0],
        RelayRequest {
            hub: "a".into(),
            node: Some("n".into()),
            target: target()
        }
    );
}

#[test]
fn replies_are_used_in_the_order_they_were_queued_then_streams_open() {
    let refusal = RelayError::Refused {
        reason: RelayRefusal::Port,
    };
    let relay = ScriptedRelay::refusing(refusal.clone());
    relay.reply(RelayReply::Refuse(RelayError::Closed));
    block_on(async {
        assert_eq!(relay.dial("h", None, &target()).await.unwrap_err(), refusal);
        assert_eq!(
            relay.dial("h", None, &target()).await.unwrap_err(),
            RelayError::Closed
        );
        assert!(relay.dial("h", None, &target()).await.is_ok());
    });
}

#[test]
fn a_hanging_dial_never_answers_and_is_counted_when_dropped() {
    let relay = ScriptedRelay::default();
    relay.reply(RelayReply::Hang);
    let mut dial = relay.dial("h", None, &target());
    assert!((&mut dial).now_or_never().is_none());
    assert_eq!(relay.abandoned_dials(), 0);
    drop(dial);
    assert_eq!(relay.abandoned_dials(), 1);
}
