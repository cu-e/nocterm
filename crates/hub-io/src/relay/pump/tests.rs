use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nocterm_hub_client::RelayError;
use nocterm_session::{RELAY_FRAME_BYTES, RELAY_FRAMES, RelayReceiver, RelaySender, RelayStream};
use tokio::sync::{Notify, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::{Downlink, Uplink, carry};

const PATIENCE: Duration = Duration::from_secs(5);

/// What the hub's stream was told, in order.
#[derive(Debug, PartialEq, Eq)]
enum Told {
    Data(Vec<u8>),
    Finished,
}

/// A hub stream going towards the target. `gate` holds sends back like a slow target does;
/// `gone` resolves when the stream is dropped, which aborts a real one.
struct FakeUp {
    told: mpsc::UnboundedSender<Told>,
    gate: Option<Arc<Semaphore>>,
    entered: Arc<Notify>,
    _gone: oneshot::Sender<()>,
}

impl Uplink for FakeUp {
    async fn send(&mut self, data: &[u8]) -> Result<(), RelayError> {
        self.entered.notify_one();
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        self.told
            .send(Told::Data(data.to_vec()))
            .map_err(|_| RelayError::Interrupted)
    }

    async fn finish(self) {
        let _ = self.told.send(Told::Finished);
    }
}

/// A hub stream coming back from the target, fed by the test.
struct FakeDown {
    script: mpsc::UnboundedReceiver<Result<Option<Vec<u8>>, RelayError>>,
    taken: Arc<AtomicUsize>,
}

impl Downlink for FakeDown {
    async fn next(&mut self) -> Result<Option<Vec<u8>>, RelayError> {
        let next = self
            .script
            .recv()
            .await
            .unwrap_or(Err(RelayError::Interrupted));
        self.taken.fetch_add(1, Ordering::SeqCst);
        next
    }
}

struct Relay {
    /// The transport's ends.
    to_hub: RelaySender,
    from_hub: RelayReceiver,
    told: mpsc::UnboundedReceiver<Told>,
    entered: Arc<Notify>,
    gone: oneshot::Receiver<()>,
    answers: mpsc::UnboundedSender<Result<Option<Vec<u8>>, RelayError>>,
    taken: Arc<AtomicUsize>,
    pump: JoinHandle<()>,
}

fn relay(gate: Option<Arc<Semaphore>>) -> Relay {
    let (stream, peer) = RelayStream::pair();
    let (told, told_rx) = mpsc::unbounded_channel();
    let (answers, script) = mpsc::unbounded_channel();
    let (gone_tx, gone) = oneshot::channel();
    let entered = Arc::new(Notify::new());
    let taken = Arc::new(AtomicUsize::new(0));
    let up = FakeUp {
        told,
        gate,
        entered: entered.clone(),
        _gone: gone_tx,
    };
    let down = FakeDown {
        script,
        taken: taken.clone(),
    };
    let (to_hub, from_hub) = stream.into_parts();
    Relay {
        to_hub,
        from_hub,
        told: told_rx,
        entered,
        gone,
        answers,
        taken,
        pump: tokio::spawn(carry(peer, up, down)),
    }
}

async fn ended(pump: JoinHandle<()>) {
    timeout(PATIENCE, pump)
        .await
        .expect("the relay did not end")
        .unwrap();
}

#[tokio::test]
async fn frames_reach_the_hub_in_order_and_an_end_of_file_is_a_half_close() {
    let mut relay = relay(None);
    relay.to_hub.send(b"one".to_vec()).await.unwrap();
    relay.to_hub.send(b"two".to_vec()).await.unwrap();
    drop(relay.to_hub);
    assert_eq!(relay.told.recv().await, Some(Told::Data(b"one".to_vec())));
    assert_eq!(relay.told.recv().await, Some(Told::Data(b"two".to_vec())));
    assert_eq!(relay.told.recv().await, Some(Told::Finished));
    // The target may still answer after the half-close.
    relay.answers.send(Ok(Some(b"late".to_vec()))).unwrap();
    assert_eq!(relay.from_hub.recv().await.unwrap(), b"late");
}

#[tokio::test]
async fn data_from_the_target_comes_in_frames_the_transport_accepts() {
    let relay = relay(None);
    let data: Vec<u8> = (0..(3 * RELAY_FRAME_BYTES + 1))
        .map(|index| (index % 251) as u8)
        .collect();
    relay.answers.send(Ok(Some(data.clone()))).unwrap();
    let mut received = Vec::new();
    while received.len() < data.len() {
        let frame = timeout(PATIENCE, relay.from_hub.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(frame.len() <= RELAY_FRAME_BYTES);
        received.extend(frame);
    }
    assert_eq!(received, data);
}

#[tokio::test]
async fn a_clean_close_by_the_target_is_end_of_file_after_the_queued_data() {
    let mut relay = relay(None);
    relay.answers.send(Ok(Some(b"bye".to_vec()))).unwrap();
    relay.answers.send(Ok(None)).unwrap();
    assert_eq!(relay.from_hub.recv().await.unwrap(), b"bye");
    assert!(relay.from_hub.recv().await.is_err(), "end of file");
    // The transport may still send: its direction has not ended.
    relay.to_hub.send(b"still".to_vec()).await.unwrap();
    assert_eq!(relay.told.recv().await, Some(Told::Data(b"still".to_vec())));
}

#[tokio::test]
async fn a_failed_hub_stream_closes_both_directions() {
    let relay = relay(None);
    relay.answers.send(Err(RelayError::Interrupted)).unwrap();
    ended(relay.pump).await;
    assert!(relay.from_hub.recv().await.is_err());
    assert!(relay.to_hub.send(b"x".to_vec()).await.is_err());
}

#[tokio::test]
async fn dropping_both_transport_ends_releases_the_hub_stream_while_the_target_is_silent() {
    let relay = relay(None);
    drop((relay.to_hub, relay.from_hub));
    ended(relay.pump).await;
    // The sender of `gone` was dropped with the stream: this resolves with an error.
    assert!(timeout(PATIENCE, relay.gone).await.unwrap().is_err());
}

#[tokio::test]
async fn a_transport_that_stops_reading_releases_the_hub_stream_while_the_target_is_silent() {
    let relay = relay(None);
    drop(relay.from_hub);
    ended(relay.pump).await;
    assert!(relay.to_hub.send(b"x".to_vec()).await.is_err());
}

#[tokio::test]
async fn a_target_that_stops_reading_stalls_the_transport_at_the_bounded_queue() {
    let gate = Arc::new(Semaphore::new(0));
    let mut relay = relay(Some(gate.clone()));
    relay.to_hub.send(vec![0]).await.unwrap();
    timeout(PATIENCE, relay.entered.notified()).await.unwrap();
    // The pump holds that frame; the channel takes `RELAY_FRAMES` more and no further.
    let mut queued = 0;
    while relay.to_hub.try_send(vec![1]).is_ok() {
        queued += 1;
    }
    assert_eq!(queued, RELAY_FRAMES);
    // Letting the target read again moves everything, in order.
    gate.add_permits(RELAY_FRAMES + 1);
    assert_eq!(relay.told.recv().await, Some(Told::Data(vec![0])));
    for _ in 0..RELAY_FRAMES {
        assert_eq!(relay.told.recv().await, Some(Told::Data(vec![1])));
    }
}

#[tokio::test]
async fn a_transport_that_stops_reading_stalls_the_hub_at_the_bounded_queue() {
    let relay = relay(None);
    for _ in 0..(10 * RELAY_FRAMES) {
        relay.answers.send(Ok(Some(vec![7]))).unwrap();
    }
    // Wait until the pump has taken what it can.
    let mut last = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let now = relay.taken.load(Ordering::SeqCst);
        if now == last {
            break;
        }
        last = now;
    }
    // `RELAY_FRAMES` in the channel, one held by the pump, and the one it asked for next.
    assert!(
        last <= RELAY_FRAMES + 2,
        "the pump took {last} frames from a hub the transport does not read"
    );
    assert!(last >= RELAY_FRAMES, "the channel was not filled: {last}");
}
