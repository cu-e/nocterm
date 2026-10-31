//! Creating a chat: material, stored grant, then the runner's claim (ADR 0122).

use std::sync::Arc;

use nocterm_cloud_agents_model::{ChatHost, OpenedChat};
use nocterm_cloud_model::identity::{ChatKeyEpoch, CloudChatId};

use super::error::{self, CloudChatError};
use super::open::open_source;
use super::session::OpenedAccount;
use super::setup::Capability;
use super::wait::PATIENCE;
use super::{Shared, managed};
use crate::cloud::ChatMaterialRequest;
use crate::devices::Captured;
use crate::hub_account::HubAccount;

/// The first key epoch of a chat; rotation is a separate transition.
const FIRST_EPOCH: u64 = 1;

pub(super) async fn create(
    shared: Arc<Shared>,
    captured: Arc<Captured>,
    account: HubAccount,
    host: ChatHost,
) -> Result<OpenedChat, CloudChatError> {
    let chat_bytes = nocterm_keys::random_bytes::<16>()
        .map_err(|failure| CloudChatError::Backend(nocterm_core::SharedCause::new(failure)))?;
    let (runner, title, capability) = match host {
        ChatHost::Agent { agent, model } => {
            let runner = managed::start_for_chat(&captured, &agent, None, chat_bytes).await?;
            (runner, agent.to_string(), Capability::Prompting { model })
        }
        ChatHost::Runner(runner) => (runner.id, runner.label, Capability::Controls),
    };
    let opened = OpenedAccount::open(&account, shared.handle(), shared.budget()).await?;
    let registration = opened.runner(&runner)?;
    let request = ChatMaterialRequest {
        stream: registration
            .binding()
            .stream(CloudChatId::from_bytes(chat_bytes)),
        epoch: ChatKeyEpoch::new(FIRST_EPOCH).map_err(|_| CloudChatError::Capacity)?,
        pins: registration.pins(),
    };
    let access = opened.material();
    let chat = Arc::new(
        access
            .create(request, shared.budget().material()?)
            .await
            .map_err(error::material)?,
    );
    // The grant bytes are durable in the account before the first publication, so a retry
    // after a crash sends the same blob.
    let grant = access
        .store_chat_key_grant(chat.clone(), shared.budget().material()?)
        .await
        .map_err(error::material)?;
    let source = open_source(
        access,
        chat,
        &capability,
        shared.collaborators(),
        shared.budget(),
        Some(Arc::new(grant)),
        PATIENCE,
    )
    .await?;
    Ok(OpenedChat { source, title })
}
