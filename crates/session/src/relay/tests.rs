use std::error::Error as _;

use futures::{FutureExt as _, executor::block_on};
use nocterm_core::SharedCause;

use super::*;

#[test]
fn both_directions_queue_a_bounded_number_of_frames() {
    let (stream, peer) = RelayStream::pair();
    let (to_relay, from_relay) = stream.into_parts();
    let (from_transport, to_transport) = peer.into_parts();
    for sender in [&to_relay, &to_transport] {
        assert_eq!(sender.capacity(), Some(RELAY_FRAMES));
        for _ in 0..RELAY_FRAMES {
            assert!(sender.try_send(vec![0]).is_ok());
        }
        assert!(
            sender.try_send(vec![0]).unwrap_err().is_full(),
            "the next frame waits for the reader"
        );
    }
    assert_eq!(from_transport.len(), RELAY_FRAMES);
    assert_eq!(from_relay.len(), RELAY_FRAMES);
}

#[test]
fn frames_keep_their_order_in_each_direction() {
    let (stream, peer) = RelayStream::pair();
    let (to_relay, from_relay) = stream.into_parts();
    let (from_transport, to_transport) = peer.into_parts();
    block_on(async {
        for frame in [b"a".to_vec(), b"b".to_vec()] {
            to_relay.send(frame).await.unwrap();
        }
        to_transport.send(b"reply".to_vec()).await.unwrap();
        assert_eq!(from_transport.recv().await.unwrap(), b"a");
        assert_eq!(from_transport.recv().await.unwrap(), b"b");
        assert_eq!(from_relay.recv().await.unwrap(), b"reply");
    });
}

#[test]
fn ending_one_direction_leaves_the_other_open_after_the_queue_drains() {
    let (stream, peer) = RelayStream::pair();
    let (to_relay, from_relay) = stream.into_parts();
    let (from_transport, to_transport) = peer.into_parts();
    block_on(async {
        to_relay.send(b"last".to_vec()).await.unwrap();
        drop(to_relay);
        assert_eq!(from_transport.recv().await.unwrap(), b"last");
        assert!(
            from_transport.recv().await.is_err(),
            "end of that direction"
        );
        to_transport.send(b"still".to_vec()).await.unwrap();
        assert_eq!(from_relay.recv().await.unwrap(), b"still");
    });
}

#[test]
fn dropping_a_whole_end_fails_the_other_ends_sends_and_reads() {
    let (stream, peer) = RelayStream::pair();
    let (to_relay, from_relay) = stream.into_parts();
    drop(peer);
    block_on(async {
        assert!(to_relay.send(vec![1]).await.is_err());
        assert!(from_relay.recv().await.is_err());
    });
}

#[test]
fn a_dialer_without_an_answer_can_be_abandoned() {
    struct Pending;
    impl RelayDialer for Pending {
        fn dial(
            &self,
            _: &str,
            _: Option<&str>,
            _: &Target,
        ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
            futures::future::pending().boxed()
        }
    }
    let target = Target::new("user", "host", 22);
    let dial = Pending.dial("hub", None, &target);
    assert!(dial.now_or_never().is_none());
}

#[test]
fn relay_errors_compare_by_what_the_caller_can_react_to() {
    let cause = |text: &str| SharedCause::new(std::io::Error::other(text.to_owned()));
    assert_eq!(
        RelayError::NotSignedIn { hub: "a".into() },
        RelayError::NotSignedIn { hub: "a".into() }
    );
    assert_ne!(
        RelayError::NotSignedIn { hub: "a".into() },
        RelayError::NotSignedIn { hub: "b".into() }
    );
    assert_ne!(
        RelayError::Refused {
            reason: RelayRefusal::Port
        },
        RelayError::Refused {
            reason: RelayRefusal::Limit
        }
    );
    assert_eq!(
        RelayError::Unreachable(cause("down")),
        RelayError::Unreachable(cause("down"))
    );
    assert_ne!(
        RelayError::Unreachable(cause("down")),
        RelayError::Unreachable(cause("slow"))
    );
    assert_ne!(
        RelayError::Closed,
        RelayError::NodeUnavailable { node: "n".into() }
    );
}

#[test]
fn an_unreachable_relay_keeps_the_original_cause() {
    let error = RelayError::Unreachable(SharedCause::new(std::io::Error::other("no route")));
    let shared = error.source().unwrap();
    assert!(shared.to_string().contains("no route"));
    let original = shared.source().unwrap();
    assert!(original.downcast_ref::<std::io::Error>().is_some());
}
