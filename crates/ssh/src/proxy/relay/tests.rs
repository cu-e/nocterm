use std::time::Duration;

use nocterm_session::{RELAY_FRAME_BYTES, RELAY_FRAMES, RelayPeer, RelayReceiver, RelaySender};
use tokio::time::timeout;

use super::*;

const WAIT: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(100);

/// The handshake's stream and the relay's two channel ends.
fn open() -> (RelayIo, RelayReceiver, RelaySender) {
    let (stream, peer): (_, RelayPeer) = RelayStream::pair();
    let (from_transport, to_transport) = peer.into_parts();
    (bridge(stream), from_transport, to_transport)
}

#[tokio::test]
async fn bytes_echo_through_the_relay_in_both_directions() {
    let (mut io, from_transport, to_transport) = open();
    let echo = tokio::spawn(async move {
        while let Ok(frame) = from_transport.recv().await {
            if to_transport.send(frame).await.is_err() {
                break;
            }
        }
    });
    io.write_all(b"hello relay").await.unwrap();
    let mut echoed = [0; 11];
    timeout(WAIT, io.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&echoed, b"hello relay");
    drop(io);
    timeout(WAIT, echo).await.unwrap().unwrap();
}

#[tokio::test]
async fn a_long_write_is_split_into_frames_the_relay_accepts() {
    let (mut io, from_transport, _to_transport) = open();
    let total = 3 * RELAY_FRAME_BYTES + 17;
    let sent: Vec<u8> = (0..total).map(|index| (index % 251) as u8).collect();
    let writer = {
        let sent = sent.clone();
        tokio::spawn(async move {
            io.write_all(&sent).await.unwrap();
            io
        })
    };
    let mut received = Vec::new();
    while received.len() < total {
        let frame = timeout(WAIT, from_transport.recv()).await.unwrap().unwrap();
        assert!(!frame.is_empty() && frame.len() <= RELAY_FRAME_BYTES);
        received.extend(frame);
    }
    assert_eq!(received, sent);
    drop(timeout(WAIT, writer).await.unwrap().unwrap());
}

#[tokio::test]
async fn a_relay_that_stops_reading_stalls_the_writer_within_the_bounds() {
    let (mut io, from_transport, _to_transport) = open();
    let chunk = vec![7; RELAY_FRAME_BYTES];
    let mut written = 0usize;
    // Everything that can be buffered: the channel, the pipe and the frame a pump holds.
    let ceiling = (RELAY_FRAMES + 3) * RELAY_FRAME_BYTES;
    loop {
        match timeout(SHORT, io.write_all(&chunk)).await {
            Ok(result) => {
                result.unwrap();
                written += chunk.len();
                assert!(
                    written <= ceiling,
                    "buffered {written} bytes without a reader"
                );
            }
            Err(_) => break,
        }
    }
    timeout(WAIT, async {
        while from_transport.len() < RELAY_FRAMES {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the channel fills to its bound");
    // Reading frees room, and the writer goes on.
    while from_transport.try_recv().is_ok() {}
    timeout(WAIT, io.write_all(&chunk)).await.unwrap().unwrap();
}

#[tokio::test]
async fn shutting_the_write_side_ends_only_that_direction() {
    let (mut io, from_transport, to_transport) = open();
    io.write_all(b"request").await.unwrap();
    io.shutdown().await.unwrap();
    assert_eq!(
        timeout(WAIT, from_transport.recv()).await.unwrap().unwrap(),
        b"request"
    );
    assert!(
        timeout(WAIT, from_transport.recv()).await.unwrap().is_err(),
        "the relay sees the end of the request"
    );
    to_transport.send(b"response".to_vec()).await.unwrap();
    let mut response = [0; 8];
    timeout(WAIT, io.read_exact(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&response, b"response");
}

#[tokio::test]
async fn the_relay_closing_its_sender_is_end_of_file_after_the_queued_bytes() {
    let (mut io, _from_transport, to_transport) = open();
    to_transport.send(b"tail".to_vec()).await.unwrap();
    drop(to_transport);
    let mut everything = Vec::new();
    timeout(WAIT, io.read_to_end(&mut everything))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(everything, b"tail");
    // The other direction is still open.
    io.write_all(b"goodbye").await.unwrap();
}

#[tokio::test]
async fn the_relay_dropping_both_ends_fails_the_writer_and_ends_the_reader() {
    let (mut io, from_transport, to_transport) = open();
    drop((from_transport, to_transport));
    let failed = timeout(WAIT, async {
        loop {
            if io.write_all(&[0; 1024]).await.is_err() {
                return;
            }
        }
    })
    .await;
    assert!(failed.is_ok(), "writes keep succeeding into a closed relay");
    let mut byte = [0];
    assert_eq!(io.read(&mut byte).await.unwrap(), 0);
}

#[tokio::test]
async fn dropping_the_stream_releases_the_relay_even_while_it_is_idle() {
    let (io, from_transport, to_transport) = open();
    drop(io);
    assert!(
        timeout(WAIT, from_transport.recv()).await.unwrap().is_err(),
        "the relay sees the end of the stream"
    );
    timeout(WAIT, async {
        while to_transport.send(vec![1]).await.is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the relay's sends fail once the stream is gone");
}
