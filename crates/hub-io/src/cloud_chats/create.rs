//! Creating a chat: material, stored grant, then the runner's claim (ADR 0122).

use std::sync::Arc;

use nocterm_cloud_agents_model::{AgentId, ChatHost, OpenedChat};
use nocterm_cloud_model::identity::{ChatKeyEpoch, CloudChatId};
use nocterm_cloud_model::projection::ports::CapturedConversationSource;
use nocterm_hubs_model::HubId;

use super::error::{self, CloudChatError};
use super::open::open_source;
use super::seat_wake::{ManagedSeat, WakingSource};
use super::session::OpenedAccount;
use super::setup::Capability;
use super::wait::PATIENCE;
use super::{Shared, managed, own_runner};
use crate::cloud::ChatMaterialRequest;
use crate::devices::Captured;
use crate::hub_account::HubAccount;

/// The first key epoch of a chat; rotation is a separate transition.
const FIRST_EPOCH: u64 = 1;

/// Where a new chat runs and what it may do there.
struct Destination {
    runner: [u8; 16],
    title: String,
    capability: Capability,
    /// The personal agent whose key this device leases to the person's own runner.
    own: Option<(AgentId, String)>,
    /// The managed agent whose seat hosts the chat and is woken for its prompts.
    seat: Option<AgentId>,
}

/// Resolves `host`; a managed agent's seat is started (or woken) for the chat first.
async fn destination(
    captured: &Arc<Captured>,
    host: ChatHost,
    chat: [u8; 16],
) -> Result<Destination, CloudChatError> {
    Ok(match host {
        ChatHost::Agent { agent, model } => Destination {
            runner: managed::start_for_chat(captured, &agent, None, chat).await?,
            title: agent.to_string(),
            capability: Capability::Prompting { model },
            own: None,
            seat: Some(agent),
        },
        ChatHost::Runner(runner) => Destination {
            runner: runner.id,
            title: runner.label,
            capability: Capability::Controls,
            own: None,
            seat: None,
        },
        ChatHost::OwnAgent {
            runner,
            agent,
            model,
        } => Destination {
            runner: runner.id,
            title: runner.label,
            capability: Capability::Controls,
            own: Some((agent, model)),
            seat: None,
        },
    })
}

pub(super) async fn create(
    shared: Arc<Shared>,
    captured: Arc<Captured>,
    account: HubAccount,
    hub: HubId,
    host: ChatHost,
) -> Result<OpenedChat, CloudChatError> {
    let chat_bytes = nocterm_keys::random_bytes::<16>()
        .map_err(|failure| CloudChatError::Backend(nocterm_core::SharedCause::new(failure)))?;
    let Destination {
        runner,
        title,
        mut capability,
        own,
        seat,
    } = destination(&captured, host, chat_bytes).await?;
    let opened = OpenedAccount::open(&account, shared.handle(), shared.budget()).await?;
    // The key and the runner's boot are checked before any chat material exists, so a missing
    // key or an offline runner leaves nothing behind.
    let prepared = match &own {
        Some((agent, model)) => {
            let prepared = own_runner::prepare(&shared, &account, &opened, agent, &runner).await?;
            capability = Capability::OwnProvider {
                provider: prepared.provider().to_owned(),
                model: model.clone(),
            };
            Some(prepared)
        }
        None => None,
    };
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
    if let (Some((agent, _)), Some(prepared)) = (&own, &prepared) {
        own_runner::lease_chat(&shared, prepared, &hub, agent, runner, &source).await?;
    }
    let source: Arc<dyn CapturedConversationSource> = source;
    // The seat of a managed agent sleeps again after its idle time while the chat stays open.
    let source = match seat {
        Some(agent) => WakingSource::wrap(
            source,
            Arc::new(ManagedSeat::new(
                shared.handle().clone(),
                captured,
                agent,
                runner,
                chat_bytes,
            )),
        ),
        None => source,
    };
    let key = super::chat_key(&chat_bytes);
    Ok(OpenedChat { source, title, key })
}
