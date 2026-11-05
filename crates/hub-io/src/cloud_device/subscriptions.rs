//! Lossless, bounded update delivery. Dropping a reader wakes its owning flow.
use super::update_size;
use futures::{
    FutureExt as _, Stream,
    future::{BoxFuture, Shared},
    stream::BoxStream,
};
use nocterm_cloud_conversation_model::{CloudConversationError, ConversationUpdate};
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};

const PENDING_UPDATES: usize = 8;
const RETAINED_BYTES: usize = 64 * 1024 * 1024;
const MAX_SUBSCRIBERS: usize = 256;
const WAITING_SUBSCRIBERS: usize = 32;
type Ended = Shared<BoxFuture<'static, ()>>;

/// One installation bounds payloads queued for all of its chats together.
pub(crate) struct UpdateBudget {
    available: Arc<Semaphore>,
    limit: usize,
}
impl Default for UpdateBudget {
    fn default() -> Self {
        Self::new(RETAINED_BYTES)
    }
}
impl UpdateBudget {
    fn new(limit: usize) -> Self {
        Self {
            available: Arc::new(Semaphore::new(limit)),
            limit,
        }
    }
}

struct Delivery {
    update: ConversationUpdate,
    _reservation: Option<OwnedSemaphorePermit>,
}
#[derive(Clone)]
pub(super) struct Subscriber(mpsc::Sender<Arc<Delivery>>);

struct Updates {
    receiver: mpsc::Receiver<Arc<Delivery>>,
    changed: Arc<Notify>,
}
impl Stream for Updates {
    type Item = ConversationUpdate;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx).map(|next| {
            next.map(|delivery| match Arc::try_unwrap(delivery) {
                Ok(delivery) => delivery.update,
                Err(shared) => shared.update.clone(),
            })
        })
    }
}
impl Drop for Updates {
    fn drop(&mut self) {
        self.receiver.close();
        self.changed.notify_one();
    }
}

pub(super) fn channel(
    changed: Arc<Notify>,
) -> (Subscriber, BoxStream<'static, ConversationUpdate>) {
    let (sender, receiver) = mpsc::channel(PENDING_UPDATES);
    (Subscriber(sender), Box::pin(Updates { receiver, changed }))
}

/// Fresh subscribers have empty queues: a terminal refusal never waits behind data.
pub(super) fn refuse(subscriber: Subscriber, error: CloudConversationError) {
    let _ = subscriber.0.try_send(Arc::new(Delivery {
        update: ConversationUpdate::Refused(error),
        _reservation: None,
    }));
}

type Pending = BoxFuture<'static, Result<Option<Subscriber>, CloudConversationError>>;

/// The original Reading owns the consumers; the channels own queued reservations.
pub(super) struct Subscriptions {
    subscribers: Vec<Subscriber>,
    waiting: VecDeque<Subscriber>,
    pending: Option<Pending>,
    budget: Arc<UpdateBudget>,
}
impl Subscriptions {
    pub(super) fn new(budget: Arc<UpdateBudget>) -> Self {
        Self {
            subscribers: Vec::new(),
            waiting: VecDeque::new(),
            pending: None,
            budget,
        }
    }
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn is_empty(&mut self) -> bool {
        self.subscribers
            .retain(|subscriber| !subscriber.0.is_closed());
        self.waiting.retain(|subscriber| !subscriber.0.is_closed());
        self.subscribers.is_empty() && self.waiting.is_empty() && !self.busy()
    }
    /// Retain only the new channel, constructing its snapshot when it can be sent.
    pub(super) fn defer(&mut self, subscriber: Subscriber) {
        self.is_empty();
        if self.waiting.len() >= WAITING_SUBSCRIBERS
            || self.subscribers.len() + self.waiting.len() + usize::from(self.busy())
                >= MAX_SUBSCRIBERS
        {
            refuse(subscriber, CloudConversationError::Capacity);
        } else {
            self.waiting.push_back(subscriber);
        }
    }
    pub(super) fn waiting(&mut self) -> Option<Subscriber> {
        if self.busy() {
            None
        } else {
            self.waiting.pop_front()
        }
    }
    pub(super) fn begin_subscribe(
        &mut self,
        subscriber: Subscriber,
        initial: [ConversationUpdate; 2],
        ended: Ended,
    ) {
        let budget = self.budget.clone();
        self.pending = Some(
            async move {
                for update in initial {
                    if let Err(error) = send(
                        std::slice::from_ref(&subscriber),
                        &budget,
                        update,
                        ended.clone(),
                    )
                    .await
                    {
                        if matches!(error, CloudConversationError::Capacity) {
                            refuse(subscriber, error);
                            return Ok(None);
                        }
                        return Err(error);
                    }
                }
                Ok(Some(subscriber))
            }
            .boxed(),
        );
    }
    pub(super) fn begin_publish(&mut self, update: ConversationUpdate, ended: Ended) {
        self.is_empty();
        let targets = self.subscribers.clone();
        let budget = self.budget.clone();
        self.pending = Some(
            async move {
                send(&targets, &budget, update, ended).await?;
                Ok(None)
            }
            .boxed(),
        );
    }
    /// The future stays in its owner when a control branch wins the actor select.
    /// In particular a partially sent broadcast resumes at the next consumer.
    pub(super) async fn advance(&mut self) -> Result<(), CloudConversationError> {
        let Some(pending) = self.pending.as_mut() else {
            return futures::future::pending().await;
        };
        let result = pending.await;
        self.pending = None;
        if let Some(subscriber) = result?
            && !subscriber.0.is_closed()
        {
            self.subscribers.push(subscriber);
        }
        Ok(())
    }
    /// Preserve the already accepted broadcast before the terminal diagnostic.
    pub(super) async fn finish(&mut self, error: CloudConversationError, ended: Ended) {
        if self.busy() && self.advance().await.is_err() {
            return;
        }
        for subscriber in self.waiting.drain(..) {
            refuse(subscriber, error.clone());
        }
        self.begin_publish(ConversationUpdate::Refused(error), ended);
        let _ = self.advance().await;
    }
    #[cfg(test)]
    async fn subscribe(
        &mut self,
        subscriber: Subscriber,
        initial: [ConversationUpdate; 2],
        ended: Ended,
    ) -> Result<(), CloudConversationError> {
        self.begin_subscribe(subscriber, initial, ended);
        self.advance().await
    }
    #[cfg(test)]
    async fn publish(
        &mut self,
        update: ConversationUpdate,
        ended: Ended,
    ) -> Result<(), CloudConversationError> {
        self.begin_publish(update, ended);
        self.advance().await
    }
}

/// A shared generation is charged once, then handed off without a deep clone for
/// the usual single consumer. A slow reader applies backpressure, never delta loss.
async fn send(
    targets: &[Subscriber],
    budget: &UpdateBudget,
    update: ConversationUpdate,
    ended: Ended,
) -> Result<(), CloudConversationError> {
    if targets.iter().all(|target| target.0.is_closed()) {
        return Ok(());
    }
    let bytes = update_size::retained(&update);
    if bytes > budget.limit {
        return Err(CloudConversationError::Capacity);
    }
    tokio::pin!(ended);
    // Channel closure is persistent. Sharing the actor's one-shot drop wake
    // could leave this wait asleep after the actor consumed that notification.
    let abandoned = async {
        for target in targets {
            target.0.closed().await;
        }
    };
    let reservation = tokio::select! {
        () = &mut ended => return Err(CloudConversationError::Stale),
        () = abandoned => return Ok(()),
        reserved = budget.available.clone().acquire_many_owned(bytes as u32) => {
            reserved.map_err(|_| CloudConversationError::Ended)?
        },
    };
    let delivery = Arc::new(Delivery {
        update,
        _reservation: Some(reservation),
    });
    for target in targets {
        tokio::select! {
            () = &mut ended => return Err(CloudConversationError::Stale),
            _ = target.0.send(delivery.clone()) => {},
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
