//! Announces a chat the runner deleted to the hub, so the hub can reclaim its capacity.
//!
//! The coordinator hands over each durable deletion through [`ChatRetirement`]; this adapter
//! signs the hub tombstone with the runner's proof key (the key epoch comes from the runner's
//! own material) and repeats it until a reader acknowledges. The work runs on the injected
//! runtime and never blocks the command order. A deletion in flight when the runner stops is
//! not resumed; the chat then stays on the hub until its retention ends.
use super::{RunnerHubPeer, RunnerMaterialStore, retire_until_compacted};
use nocterm_cloud_model::coordinator::{ChatRetirement, ChatRetirementRequest};
use std::{sync::Arc, time::Duration};
use tokio::runtime::Handle;

/// How often the tombstone is repeated while a reader has not acknowledged it.
const RETIRE_INTERVAL: Duration = Duration::from_secs(60);
/// A day of attempts; afterwards the hub's retention ends the stream.
const RETIRE_ATTEMPTS: u32 = 24 * 60;

pub struct RunnerChatRetirement {
    peer: RunnerHubPeer,
    material: Arc<RunnerMaterialStore>,
    handle: Handle,
}

impl RunnerChatRetirement {
    pub fn new(peer: RunnerHubPeer, material: Arc<RunnerMaterialStore>, handle: Handle) -> Self {
        Self {
            peer,
            material,
            handle,
        }
    }
}

impl ChatRetirement for RunnerChatRetirement {
    fn retire(&self, request: ChatRetirementRequest) {
        let Some(epoch) = self.material.chat_key_epoch(request.stream()) else {
            tracing::debug!("a deleted chat has no live key material; it is not announced");
            return;
        };
        let peer = self.peer.clone();
        let chat = *request.stream().chat().as_bytes();
        let device = request.requested_by().as_str().to_owned();
        self.handle.spawn(async move {
            let outcome = retire_until_compacted(
                || peer.retire_chat(chat, epoch.get(), &device),
                RETIRE_INTERVAL,
                RETIRE_ATTEMPTS,
            )
            .await;
            if let Err(error) = outcome {
                tracing::debug!(%error, "the deletion of a chat was not announced to the hub");
            }
        });
    }
}
