use super::lock;
use crate::{RelayDialer, RelayError, RelayPeer, RelayStream, Target};
use async_channel::{Receiver, Sender, unbounded};
use futures::{FutureExt as _, future::BoxFuture};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

/// What one dial asked of the relay: the hub, its node and the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayRequest {
    pub hub: String,
    pub node: Option<String>,
    pub target: Target,
}

/// How the next dial ends.
#[derive(Debug, Clone)]
pub enum RelayReply {
    /// Opens a stream; the relay's end of it is given out by [`ScriptedRelay::next_peer`].
    Open,
    /// Fails with this error.
    Refuse(RelayError),
    /// Never answers; [`ScriptedRelay::abandoned_dials`] counts the futures dropped.
    Hang,
}

struct Relay {
    replies: Mutex<VecDeque<RelayReply>>,
    asked: Mutex<Vec<RelayRequest>>,
    abandoned: AtomicUsize,
    give: Sender<RelayPeer>,
    peers: Receiver<RelayPeer>,
}

/// A [`RelayDialer`] answering from a script: replies are used in the order they were
/// queued and an empty script opens a stream. Every request is recorded when `dial` is
/// called. Clones share one script.
#[derive(Clone)]
pub struct ScriptedRelay(Arc<Relay>);

impl Default for ScriptedRelay {
    fn default() -> Self {
        let (give, peers) = unbounded();
        Self(Arc::new(Relay {
            replies: Mutex::default(),
            asked: Mutex::default(),
            abandoned: AtomicUsize::new(0),
            give,
            peers,
        }))
    }
}

impl ScriptedRelay {
    /// A relay whose next dial is refused with `error`.
    pub fn refusing(error: RelayError) -> Self {
        let relay = Self::default();
        relay.reply(RelayReply::Refuse(error));
        relay
    }

    /// Queues the outcome of one more dial.
    pub fn reply(&self, reply: RelayReply) {
        lock(&self.0.replies).push_back(reply);
    }

    /// The requests made so far, oldest first.
    pub fn asked(&self) -> Vec<RelayRequest> {
        lock(&self.0.asked).clone()
    }

    /// Waits for the relay's end of the next opened stream.
    pub async fn next_peer(&self) -> RelayPeer {
        self.0
            .peers
            .recv()
            .await
            .expect("the relay keeps its own sender")
    }

    /// How many dials that never answered were dropped before they finished.
    pub fn abandoned_dials(&self) -> usize {
        self.0.abandoned.load(Ordering::SeqCst)
    }
}

/// Counts the dial future that is dropped while it waits.
struct Abandoned(Arc<Relay>);

impl Drop for Abandoned {
    fn drop(&mut self) {
        self.0.abandoned.fetch_add(1, Ordering::SeqCst);
    }
}

impl RelayDialer for ScriptedRelay {
    fn capture(
        &self,
        _hub: &str,
        _node: Option<&str>,
        _target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        Ok(capture_relay(Arc::new(self.clone()), _hub, _node, _target))
    }

    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        lock(&self.0.asked).push(RelayRequest {
            hub: hub.to_owned(),
            node: node.map(str::to_owned),
            target: target.clone(),
        });
        let reply = lock(&self.0.replies)
            .pop_front()
            .unwrap_or(RelayReply::Open);
        match reply {
            RelayReply::Open => {
                let (stream, peer) = RelayStream::pair();
                let _ = self.0.give.try_send(peer);
                async move { Ok(stream) }.boxed()
            }
            RelayReply::Refuse(error) => async move { Err(error) }.boxed(),
            RelayReply::Hang => {
                let guard = Abandoned(self.0.clone());
                async move {
                    let _guard = guard;
                    futures::future::pending().await
                }
                .boxed()
            }
        }
    }
}

/// Bind an existing shared relay fake to one immutable route. Consumers reuse
/// this view rather than implementing a second capture protocol in each suite.
pub fn capture_relay(
    inner: Arc<dyn RelayDialer>,
    hub: &str,
    node: Option<&str>,
    target: &Target,
) -> Arc<dyn RelayDialer> {
    Arc::new(CapturedRelay {
        inner,
        request: RelayRequest {
            hub: hub.to_owned(),
            node: node.map(str::to_owned),
            target: target.clone(),
        },
    })
}
#[derive(Clone)]
struct CapturedRelay {
    inner: Arc<dyn RelayDialer>,
    request: RelayRequest,
}
impl CapturedRelay {
    fn matches(&self, hub: &str, node: Option<&str>, target: &Target) -> bool {
        self.request.hub == hub
            && self.request.node.as_deref() == node
            && self.request.target == *target
    }
}
impl RelayDialer for CapturedRelay {
    fn capture(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        if !self.matches(hub, node, target) {
            return Err(RelayError::CaptureUnsupported);
        }
        Ok(Arc::new(self.clone()))
    }
    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        if !self.matches(hub, node, target) {
            return async { Err(RelayError::CaptureUnsupported) }.boxed();
        }
        self.inner.dial(hub, node, target)
    }
}
