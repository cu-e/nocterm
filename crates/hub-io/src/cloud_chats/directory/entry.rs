//! One listed chat. Holding it is the only way to open it.

use std::sync::Arc;

use nocterm_cloud_agents_model::{ChatFuture, ChatHistoryEntry, ChatSummary, OpenedChat};
use nocterm_cloud_model::journal::CurrentChatHead;
use nocterm_cloud_model::projection::ports::CapturedConversationSource;

use crate::cloud::{ChatMaterialRequest, CloudAccountMaterial, RunnerRegistration};
use crate::cloud_chats::Shared;
use crate::cloud_chats::error::{self, CloudChatError};
use crate::cloud_chats::managed::{self, Seat};
use crate::cloud_chats::open::open_source;
use crate::cloud_chats::seat_wake::{ManagedSeat, WakingSource};
use crate::cloud_chats::setup::Capability;
use crate::cloud_chats::wait::PATIENCE;
use crate::devices::Captured;

pub(super) struct Entry {
    shared: Arc<Shared>,
    captured: Arc<Captured>,
    access: Arc<CloudAccountMaterial>,
    request: ChatMaterialRequest,
    seat: Option<Seat>,
    summary: ChatSummary,
}

/// What the row of a chat shows: the catalog carries no title or time, only the chat's
/// identifier, so the row is named after its host and the start of that identifier.
fn summary(chat: &[u8; 16], host: String, model: Option<String>) -> ChatSummary {
    let key = crate::cloud_chats::chat_key(chat);
    ChatSummary {
        title: format!("{host} · {}", &key[..6]),
        key,
        host,
        model,
        updated_at: None,
    }
}

impl Entry {
    pub(super) fn new(
        shared: Arc<Shared>,
        captured: Arc<Captured>,
        access: Arc<CloudAccountMaterial>,
        registration: &RunnerRegistration,
        head: &CurrentChatHead,
        seat: Option<Seat>,
    ) -> Self {
        let host = seat
            .as_ref()
            .map_or_else(|| registration.label().to_owned(), |seat| seat.name.clone());
        let model = seat.as_ref().map(|seat| seat.model.clone());
        Self {
            summary: summary(head.stream().chat().as_bytes(), host, model),
            shared,
            captured,
            access,
            request: ChatMaterialRequest {
                stream: head.stream().clone(),
                epoch: head.key_epoch(),
                pins: registration.pins(),
            },
            seat,
        }
    }
}

impl ChatHistoryEntry for Entry {
    fn summary(&self) -> &ChatSummary {
        &self.summary
    }

    fn open(&self) -> ChatFuture<OpenedChat> {
        let shared = self.shared.clone();
        let captured = self.captured.clone();
        let access = self.access.clone();
        let request = self.request.clone();
        let chat_id = *request.stream.chat().as_bytes();
        let seat = self.seat.clone();
        let title = self.summary.title.clone();
        let key = self.summary.key.clone();
        let work = {
            let captured = captured.clone();
            async move {
                // A sleeping seat is woken first; waiting for it uses the same bounded loop.
                let (capability, patience) = match &seat {
                    Some(seat) => {
                        managed::start_for_chat(&captured, &seat.agent, Some(seat.runner), chat_id)
                            .await?;
                        (
                            Capability::Prompting {
                                model: seat.model.clone(),
                            },
                            PATIENCE,
                        )
                    }
                    None => (Capability::Controls, std::time::Duration::ZERO),
                };
                let chat = Arc::new(
                    access
                        .open(request, shared.budget().material()?)
                        .await
                        .map_err(error::material)?,
                );
                let source: Arc<dyn CapturedConversationSource> = open_source(
                    &access,
                    chat,
                    &capability,
                    shared.collaborators(),
                    shared.budget(),
                    None,
                    patience,
                )
                .await?;
                // The seat sleeps again after its idle time while the chat stays open.
                let source = match seat {
                    Some(seat) => WakingSource::wrap(
                        source,
                        Arc::new(ManagedSeat::new(
                            shared.handle().clone(),
                            captured.clone(),
                            seat.agent,
                            seat.runner,
                            chat_id,
                        )),
                    ),
                    None => source,
                };
                Ok::<_, CloudChatError>(OpenedChat { source, title, key })
            }
        };
        self.shared.run(&captured, work)
    }
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn a_row_is_named_after_its_host_and_keyed_by_the_whole_chat_identifier() {
        let mut chat = [0u8; 16];
        chat[..3].copy_from_slice(&[0x02, 0xc0, 0x73]);
        let row = summary(&chat, "First".into(), Some("gpt-x".into()));
        assert_eq!(row.title, "First · 02c073");
        assert_eq!(row.key.len(), 32);
        assert_eq!(row.host, "First");
        assert_eq!(row.model.as_deref(), Some("gpt-x"));
        assert_eq!(row.updated_at, None, "the catalog carries no time");
    }
}
