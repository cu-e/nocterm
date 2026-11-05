//! One bounded conversation mailbox owns request admission, first-worker startup and close.
//! Closing checks the final queued request under the same lock as a concurrent send.
use super::{
    prompt,
    server_flows::BindReply,
    subscriptions::{self, Subscriber},
};
use futures::channel::oneshot;
use nocterm_cloud_conversation_model::{CloudConversationError, CommandKey, ConversationAction};
use nocterm_cloud_device::ssh::SshServerChoice;
use nocterm_hub_model::cloud::ssh::MAX_BINDING_TARGETS;
use std::sync::{Mutex, PoisonError};
use tokio::sync::mpsc;

pub(super) enum Request {
    Subscribe(Subscriber),
    Deliver(
        ConversationAction,
        oneshot::Sender<Result<CommandKey, CloudConversationError>>,
    ),
    Refresh,
    ProviderRevision(oneshot::Sender<Result<u64, CloudConversationError>>),
    Bind(Vec<SshServerChoice>, i64, BindReply),
    Unbind(oneshot::Sender<Result<(), CloudConversationError>>),
}

impl Request {
    fn bound(&mut self) -> Result<(), CloudConversationError> {
        match self {
            Self::Deliver(action, _) => prompt::bound(action)?,
            Self::Bind(choices, _, _) => {
                if choices.len() > MAX_BINDING_TARGETS {
                    return Err(CloudConversationError::Capacity);
                }
                if choices.capacity() > MAX_BINDING_TARGETS {
                    *choices = std::mem::take(choices).into_boxed_slice().into_vec();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Mailbox(Mutex<Option<mpsc::Sender<Request>>>);
impl Mailbox {
    pub(super) fn send(
        &self,
        mut request: Request,
        start: impl FnOnce(mpsc::Receiver<Request>),
    ) -> Result<(), CloudConversationError> {
        request.bound()?;
        let mut mailbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let request = match mailbox.as_ref() {
            Some(sender) => match sender.try_send(request) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Closed(request)) => request,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    return Err(CloudConversationError::Capacity);
                }
            },
            None => request,
        };
        let (sender, receiver) = mpsc::channel(32);
        sender
            .try_send(request)
            .map_err(|_| CloudConversationError::Ended)?;
        *mailbox = Some(sender);
        start(receiver);
        Ok(())
    }
    pub(super) fn send_existing(&self, mut request: Request) -> Result<(), CloudConversationError> {
        request.bound()?;
        let mailbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = mailbox.as_ref() {
            sender
                .try_send(request)
                .map_err(|_| CloudConversationError::Capacity)?;
        }
        Ok(())
    }
    pub(super) fn finish(&self, requests: &mut mpsc::Receiver<Request>) -> Option<Request> {
        let mut mailbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match requests.try_recv() {
            Ok(request) => Some(request),
            _ => {
                *mailbox = None;
                None
            }
        }
    }
    pub(super) fn refuse_all(
        &self,
        mut requests: mpsc::Receiver<Request>,
        error: &CloudConversationError,
    ) {
        let waiting = {
            let mut mailbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let mut waiting = Vec::new();
            while let Ok(request) = requests.try_recv() {
                waiting.push(request);
            }
            *mailbox = None;
            waiting
        };
        for request in waiting {
            request.refuse(error);
        }
    }
}
impl Request {
    fn refuse(self, error: &CloudConversationError) {
        match self {
            Self::Subscribe(subscriber) => subscriptions::refuse(subscriber, error.clone()),
            Self::Deliver(_, reply) => {
                let _ = reply.send(Err(error.clone()));
            }
            Self::Bind(_, _, reply) | Self::Unbind(reply) => {
                let _ = reply.send(Err(error.clone()));
            }
            Self::ProviderRevision(reply) => {
                let _ = reply.send(Err(error.clone()));
            }
            Self::Refresh => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, mpsc as std_mpsc};

    #[test]
    fn a_send_racing_the_final_idle_check_always_reaches_an_old_or_new_worker() {
        for _ in 0..16 {
            let mailbox = Arc::new(Mailbox::default());
            let mut started = None;
            mailbox
                .send(Request::Refresh, |receiver| started = Some(receiver))
                .unwrap();
            let mut requests = started.unwrap();
            assert!(matches!(requests.try_recv(), Ok(Request::Refresh)));
            let barrier = Arc::new(Barrier::new(2));
            let (new_worker, restarted) = std_mpsc::channel();
            let (reply, answer) = oneshot::channel();
            let sender = {
                let (mailbox, barrier) = (mailbox.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    mailbox
                        .send(Request::ProviderRevision(reply), |receiver| {
                            new_worker.send(receiver).unwrap()
                        })
                        .unwrap();
                })
            };
            barrier.wait();
            let received = mailbox.finish(&mut requests);
            sender.join().unwrap();
            let received =
                received.unwrap_or_else(|| restarted.recv().unwrap().try_recv().unwrap());
            let Request::ProviderRevision(reply) = received else {
                panic!("the request must reach a worker")
            };
            reply.send(Ok(42)).unwrap();
            assert!(matches!(futures::executor::block_on(answer), Ok(Ok(42))));
        }
    }
}
