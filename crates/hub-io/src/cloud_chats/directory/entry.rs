//! One listed chat. Holding it is the only way to open it.

use std::sync::Arc;

use nocterm_cloud_agents_model::{ChatFuture, ChatHistoryEntry, OpenedChat};
use nocterm_cloud_model::journal::CurrentChatHead;

use crate::cloud::{ChatMaterialRequest, CloudAccountMaterial, RunnerRegistration};
use crate::cloud_chats::Shared;
use crate::cloud_chats::error::{self, CloudChatError};
use crate::cloud_chats::managed::{self, Seat};
use crate::cloud_chats::open::open_source;
use crate::cloud_chats::setup::Capability;
use crate::cloud_chats::wait::PATIENCE;
use crate::devices::Captured;

pub(super) struct Entry {
    shared: Arc<Shared>,
    captured: Arc<Captured>,
    access: Arc<CloudAccountMaterial>,
    request: ChatMaterialRequest,
    seat: Option<Seat>,
    label: String,
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
        let chat = head.stream().chat();
        let short: String = chat
            .as_bytes()
            .iter()
            .take(3)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let host = seat
            .as_ref()
            .map_or_else(|| registration.label().to_owned(), |seat| seat.name.clone());
        Self {
            shared,
            captured,
            access,
            request: ChatMaterialRequest {
                stream: head.stream().clone(),
                epoch: head.key_epoch(),
                pins: registration.pins(),
            },
            seat,
            label: format!("{host} · {short}"),
        }
    }
}

impl ChatHistoryEntry for Entry {
    fn label(&self) -> &str {
        &self.label
    }

    fn open(&self) -> ChatFuture<OpenedChat> {
        let shared = self.shared.clone();
        let captured = self.captured.clone();
        let access = self.access.clone();
        let request = self.request.clone();
        let seat = self.seat.clone();
        let title = self.label.clone();
        let work = {
            let captured = captured.clone();
            async move {
                // A sleeping seat is woken first; waiting for it uses the same bounded loop.
                let (capability, patience) = match &seat {
                    Some(seat) => {
                        managed::start_for_chat(
                            &captured,
                            &seat.agent,
                            Some(seat.runner),
                            *request.stream.chat().as_bytes(),
                        )
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
                let source = open_source(
                    &access,
                    chat,
                    &capability,
                    shared.collaborators(),
                    shared.budget(),
                    None,
                    patience,
                )
                .await?;
                Ok::<_, CloudChatError>(OpenedChat { source, title })
            }
        };
        self.shared.run(&captured, work)
    }
}
