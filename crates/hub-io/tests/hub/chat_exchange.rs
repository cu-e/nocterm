//! One prompt and its reply through an opened Cloud chat, without a window.
//!
//! The chat entity drives the same conversation port this way: it subscribes to the verified
//! transcript the device core keeps, delivers a prompt, allows its run once it waits for its
//! grant (the core asks for that itself) and reads the runner's reply from the deltas.
use futures::StreamExt as _;
use nocterm_cloud_conversation_model::{
    CloudConversation, CloudTranscript, ConversationAction, ConversationUpdate, RowRole,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Sends `prompt` and returns the assistant's reply once the runner has published one, or
/// panics with the transcript when none arrives within `patience`.
pub(crate) fn reply_to(
    conversation: &Arc<dyn CloudConversation>,
    prompt: &str,
    patience: Duration,
) -> String {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut updates = conversation.subscribe().expect("the chat subscribes");
            conversation
                .deliver(ConversationAction::Prompt {
                    text: prompt.to_owned(),
                })
                .await
                .expect("the prompt is tracked");
            let mut transcript = CloudTranscript::new();
            let deadline = Instant::now() + patience;
            while Instant::now() < deadline {
                let next = tokio::time::timeout(Duration::from_secs(10), updates.next()).await;
                if let Ok(Some(ConversationUpdate::Transcript(delta))) = next {
                    transcript.apply(delta);
                }
                let reply: String = transcript
                    .rows()
                    .iter()
                    .filter(|row| row.role == RowRole::Assistant)
                    .map(|row| row.text.as_str())
                    .collect();
                if !reply.trim().is_empty() {
                    return reply;
                }
            }
            panic!(
                "no reply within {patience:?}; the chat holds {:?} and runs {:?}",
                transcript.rows(),
                transcript.runs()
            );
        })
}
