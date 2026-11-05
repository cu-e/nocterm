//! Creating a chat with the device core's `CreateChat` flow (ADR 0122): a managed agent's seat
//! is started for the new chat, or one of the person's own runners is named; the chat material
//! and its sealed grant are compare-created in the account, the grant is published, and the
//! flow waits (bounded, republishing) for the runner to take the chat.

use std::sync::Arc;

use nocterm_cloud_agents_model::{ChatHost, CloudChatError, OpenedChat};
use nocterm_cloud_conversation_model::CloudConversation as _;
use nocterm_cloud_device::create::{ChatHost as CoreHost, CreateChat};
use nocterm_cloud_device::open::ModelChoice;
use nocterm_cloud_device::policy::MANAGED_PROVIDER;
use nocterm_hubs_model::HubId;

use super::leases::{self, LeaseRequest};
use super::{Shared, error};
use crate::cloud_device::{ChatSetup, driver};
use crate::devices::Captured;
use crate::hub_account::HubAccount;

/// What a new chat runs on, and what its window prompts with.
struct Plan {
    host: CoreHost,
    title: String,
    model: Option<ModelChoice>,
    models: Vec<nocterm_cloud_conversation_model::ModelName>,
    wake: Option<String>,
    /// The personal agent whose key this device leases to the person's own runner.
    lease: Option<nocterm_cloud_agents_model::AgentId>,
}

async fn plan(
    captured: &Arc<Captured>,
    account: &HubAccount,
    host: ChatHost,
) -> Result<Plan, CloudChatError> {
    Ok(match host {
        ChatHost::Agent { agent, model } => {
            let catalog = crate::agents::list_agents(captured.clone()).await?;
            let models = catalog
                .iter()
                .find(|entry| entry.id == agent)
                .map(|entry| {
                    entry
                        .models
                        .iter()
                        .map(|model| nocterm_cloud_conversation_model::ModelName {
                            provider: MANAGED_PROVIDER.to_owned(),
                            model: model.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Plan {
                models,
                host: CoreHost::Managed {
                    agent_id: agent.to_string(),
                },
                title: agent.to_string(),
                model: Some(ModelChoice {
                    provider: MANAGED_PROVIDER.to_owned(),
                    model,
                }),
                wake: Some(agent.to_string()),
                lease: None,
            }
        }
        ChatHost::Runner(runner) => Plan {
            host: CoreHost::Own { runner: runner.id },
            title: runner.label,
            model: None,
            models: Vec::new(),
            wake: None,
            lease: None,
        },
        // The key is checked before any chat material exists, so a missing key leaves nothing
        // behind.
        ChatHost::OwnAgent {
            runner,
            agent,
            model,
        } => {
            let models = leases::key_models(account, &agent).await?;
            let selected = models
                .iter()
                .find(|choice| choice.model == model)
                .cloned()
                .ok_or(nocterm_cloud_conversation_model::CloudConversationError::NoModel)?;
            Plan {
                models: models
                    .into_iter()
                    .map(|model| nocterm_cloud_conversation_model::ModelName {
                        provider: model.provider,
                        model: model.model,
                    })
                    .collect(),
                host: CoreHost::Own { runner: runner.id },
                title: runner.label,
                model: Some(selected),
                wake: None,
                lease: Some(agent),
            }
        }
    })
}

pub(super) async fn create(
    shared: &Arc<Shared>,
    captured: Arc<Captured>,
    account: &HubAccount,
    hub: &HubId,
    host: ChatHost,
) -> Result<OpenedChat, CloudChatError> {
    let plan = plan(&captured, account, host).await?;
    let opened = shared.account(captured, account).await?;
    let device = opened.device().map_err(error::hub)?;
    let mut flow =
        CreateChat::new(opened.session(), device, plan.host, None).map_err(error::device)?;
    let mut created = None;
    driver::run(&mut flow, opened.effects().clone(), |_, chat| {
        created = Some(chat);
        driver::Control::Stop
    })
    .await
    .map_err(error::device)?;
    let created = created.ok_or(CloudChatError::RunnerNotReady)?;
    if let Some(runners) = created.runners {
        opened.adopt_runners(runners);
    }
    let conversation = shared.conversation(
        opened.clone(),
        hub,
        ChatSetup {
            chat: created.chat,
            model: plan.model,
            models: plan.models,
            wake: plan.wake,
            servers: None,
            providers: None,
        },
    );
    if let Some(agent) = plan.lease {
        let request = LeaseRequest {
            hub,
            agent: &agent,
            chat: created.chat,
            control_revision: 0,
        };
        let lease = leases::lease(shared, opened, request, Arc::downgrade(&conversation)).await?;
        conversation.adopt_own_lease(&lease)?;
    }
    let key = conversation.key().to_string();
    Ok(OpenedChat {
        conversation,
        title: plan.title,
        key,
    })
}
