//! Leasing the key of a personal agent to the person's own runner (ADR 0126), by the device
//! core's `ProviderLease` flow.
//!
//! The key stays in the account's encrypted store. The flow of one chat reads it, seals it to
//! the runner's verified boot and sends the capsule as a command of that chat, then renews it
//! at half term on its own; no plaintext key outlives a renewal. A flow runs while its chat is
//! open and its account is unlocked. The service's renewal asks how long the shortest lease of
//! an agent still lasts.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use nocterm_cloud_agents_model::{AgentId, CloudAgentError};
use nocterm_cloud_device::context::ChatRef;
use nocterm_cloud_device::open::ModelChoice;
use nocterm_cloud_device::provider::{Lease, ProviderLease};
use nocterm_hubs_model::HubId;

use super::error::{self, CloudChatError};
use crate::cloud_device::{DeviceAccount, DeviceConversation, driver};
use crate::hub_account::HubAccount;

/// How long one lease lasts.
pub(super) const LEASE_TERM: Duration =
    Duration::from_millis(nocterm_cloud_device::provider::LEASE_TERM_MS.unsigned_abs());
/// How often a lease task looks whether its chat is still open.
const LIVENESS: Duration = Duration::from_secs(30);

/// A chat that holds a lease of one agent's key.
struct Held {
    expires_at_ms: Arc<AtomicI64>,
    chat: Weak<DeviceConversation>,
}

struct Tracked {
    runner: [u8; 16],
    chats: Vec<Held>,
}

/// The agents whose keys this device leases, and the chats that hold such a lease.
#[derive(Default)]
pub(super) struct Leases {
    tracked: Mutex<HashMap<(HubId, AgentId), Tracked>>,
}

impl Leases {
    /// Starts tracking `agent` on `runner`; chats attached earlier stay.
    pub(super) fn track(&self, hub: &HubId, agent: &AgentId, runner: [u8; 16]) {
        self.tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((hub.clone(), agent.clone()))
            .and_modify(|entry| entry.runner = runner)
            .or_insert(Tracked {
                runner,
                chats: Vec::new(),
            });
    }
    fn attach(&self, hub: &HubId, agent: &AgentId, runner: [u8; 16], held: Held) {
        self.tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((hub.clone(), agent.clone()))
            .or_insert(Tracked {
                runner,
                chats: Vec::new(),
            })
            .chats
            .push(held);
    }
    /// How long the shortest live lease of `agent` lasts; a full term when no chat holds one.
    /// Fails with `LeaseEnded` when the agent is not tracked.
    pub(super) fn remaining(
        &self,
        hub: &HubId,
        agent: &AgentId,
    ) -> Result<Duration, CloudChatError> {
        let mut tracked = self.tracked.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = tracked
            .get_mut(&(hub.clone(), agent.clone()))
            .ok_or(CloudChatError::Agent(CloudAgentError::LeaseEnded))?;
        entry.chats.retain(|held| held.chat.strong_count() > 0);
        let now = driver::now_ms();
        Ok(entry
            .chats
            .iter()
            .map(|held| {
                held.expires_at_ms
                    .load(Ordering::Acquire)
                    .saturating_sub(now)
            })
            .filter(|left| *left > 0)
            .min()
            .map_or(LEASE_TERM, |left| {
                Duration::from_millis(left.unsigned_abs()).min(LEASE_TERM)
            }))
    }
}

/// The provider and model of the key the account holds for `agent`, which an own-runner prompt
/// names; `NoProviderKey` when it holds none.
pub(super) async fn key_model(
    account: &HubAccount,
    agent: &AgentId,
) -> Result<ModelChoice, CloudChatError> {
    let key = own_key(account, agent).await?;
    Ok(ModelChoice {
        provider: key.provider().to_owned(),
        model: key.model().to_owned(),
    })
}

/// Every model the own agent's canonical encrypted record authorizes leasing.
pub(super) async fn key_models(
    account: &HubAccount,
    agent: &AgentId,
) -> Result<Vec<ModelChoice>, CloudChatError> {
    let key = own_key(account, agent).await?;
    Ok(key
        .models()
        .iter()
        .map(|model| ModelChoice {
            provider: key.provider().to_owned(),
            model: model.clone(),
        })
        .collect())
}

async fn own_key(
    account: &HubAccount,
    agent: &AgentId,
) -> Result<crate::agents::own_keys::OwnAgentKey, CloudChatError> {
    let work = account.work().map_err(error::hub)?;
    match crate::agents::own_keys::get(&work, agent.as_str()).await {
        Ok(Some(key)) => Ok(key),
        Ok(None) => Err(CloudChatError::NoProviderKey),
        Err(CloudAgentError::Hub(error)) => Err(error::hub(error)),
        Err(other) => Err(CloudChatError::Agent(other)),
    }
}

/// What starting a chat's lease needs.
pub(super) struct LeaseRequest<'a> {
    pub(super) hub: &'a HubId,
    pub(super) agent: &'a AgentId,
    pub(super) chat: ChatRef,
    pub(super) control_revision: u64,
}

/// Starts the lease flow of `chat` and returns its first lease; the flow then renews on its own
/// for as long as `held` is open and the account unlocked.
pub(super) async fn lease(
    shared: &Arc<super::Shared>,
    account: Arc<DeviceAccount>,
    request: LeaseRequest<'_>,
    held: Weak<DeviceConversation>,
) -> Result<Lease, CloudChatError> {
    let device = account.device().map_err(error::hub)?;
    let mut flow = ProviderLease::new(
        account.session().clone(),
        device,
        &request.chat,
        request.agent.as_str(),
        request.control_revision,
    )
    .map_err(error::device)?;
    let (first, leased) = oneshot::channel();
    let expires = Arc::new(AtomicI64::new(0));
    let renewed = expires.clone();
    let liveness = held.clone();
    let refreshing = held.clone();
    let effects = account.effects().clone();
    let ended = account.clone().ended();
    shared.handle.spawn(async move {
        let mut first = Some(first);
        let running = driver::run(&mut flow, effects, |_, lease: Lease| {
            renewed.store(lease.expires_at_ms, Ordering::Release);
            if let Some(first) = first.take() {
                let _ = first.send(Ok(lease));
            } else if let Some(chat) = refreshing.upgrade()
                && let Err(error) = chat.refresh_own_lease(&lease)
            {
                tracing::debug!(%error, "the renewed provider's model projection was not accepted");
            }
            driver::Control::Continue
        });
        let open = async {
            while liveness.strong_count() > 0 {
                tokio::time::sleep(LIVENESS).await;
            }
        };
        tokio::select! {
            result = running => {
                if let (Some(first), Err(error)) = (first, result) {
                    let _ = first.send(Err(error));
                }
            }
            () = ended => {}
            () = open => {}
        }
    });
    let lease = leased
        .await
        .map_err(|_| CloudChatError::Stale)?
        .map_err(error::device)?;
    let runner = request.chat.runner;
    shared.leases.attach(
        request.hub,
        request.agent,
        runner,
        Held {
            expires_at_ms: expires,
            chat: held,
        },
    );
    Ok(lease)
}

/// The service's view of a failure: typed, with the original cause kept where there is no
/// matching variant.
pub(super) fn agent_error(error: CloudChatError) -> CloudAgentError {
    match error {
        CloudChatError::Hub(error) => CloudAgentError::Hub(error),
        CloudChatError::Stale => CloudAgentError::Stale,
        CloudChatError::Agent(error) => error,
        CloudChatError::NoProviderKey => CloudAgentError::NoOwnKey,
        CloudChatError::NotEnrolled => CloudAgentError::NoOwnRunner,
        CloudChatError::Capacity | CloudChatError::RunnerNotReady => CloudAgentError::TimedOut,
        other => CloudAgentError::Registration(nocterm_core::SharedCause::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub(name: &str) -> HubId {
        HubId::from(nocterm_hubs_model::HubAddress::parse(name).unwrap())
    }

    #[test]
    fn an_agent_that_was_never_started_has_no_lease_to_renew() {
        let leases = Leases::default();
        let (hub, agent) = (hub("https://one.example"), AgentId::new("a1").unwrap());
        assert!(matches!(
            leases.remaining(&hub, &agent),
            Err(CloudChatError::Agent(CloudAgentError::LeaseEnded))
        ));
        leases.track(&hub, &agent, [1; 16]);
        assert_eq!(leases.remaining(&hub, &agent).unwrap(), LEASE_TERM);
        // Another hub does not share the lease.
        assert!(
            leases
                .remaining(&self::hub("https://two.example"), &agent)
                .is_err()
        );
    }

    #[test]
    fn the_shortest_live_lease_says_when_to_renew() {
        let leases = Leases::default();
        let (hub, agent) = (hub("https://one.example"), AgentId::new("a1").unwrap());
        leases.track(&hub, &agent, [1; 16]);
        // A chat that closed holds no lease.
        leases.attach(
            &hub,
            &agent,
            [1; 16],
            Held {
                expires_at_ms: Arc::new(AtomicI64::new(driver::now_ms() + 1_000)),
                chat: Weak::new(),
            },
        );
        assert_eq!(leases.remaining(&hub, &agent).unwrap(), LEASE_TERM);
    }

    #[test]
    fn a_failure_of_the_lease_reaches_the_service_with_its_meaning() {
        assert!(matches!(
            agent_error(CloudChatError::NoProviderKey),
            CloudAgentError::NoOwnKey
        ));
        assert!(matches!(
            agent_error(CloudChatError::NotEnrolled),
            CloudAgentError::NoOwnRunner
        ));
        assert!(agent_error(CloudChatError::RunnerNotReady).is_transient());
    }
}
