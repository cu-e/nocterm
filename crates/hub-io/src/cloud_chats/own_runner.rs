//! Leasing the key of a personal agent to the person's own runner (ADR 0126).
//!
//! The key stays in the account's encrypted store. Each lease reads it, seals it to the
//! runner's verified boot key and hands the capsule to the runner as a command of one chat; no
//! plaintext key outlives that call. Chats that hold a lease are tracked weakly so the service's
//! renewal sends them a fresh capsule before the old one ends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use nocterm_cloud_agents_model::{AgentId, CloudAgentError, OwnRunnerSeat};
use nocterm_cloud_model::{identity::Timestamp, projection::{ProjectionError, ProviderAccess}};
use nocterm_hubs_model::HubId;

use super::error::{self, CloudChatError};
use super::session::OpenedAccount;
use super::Shared;
use crate::agents::own_keys::{self, OwnAgentKey};
use crate::cloud::{
    AccountProviderBoot, CloudConversationSource, LeaseRequest, TrustedProviderBoot,
};
use crate::hub_account::HubAccount;

/// How long one lease lasts when the runner's boot allows it.
pub(super) const LEASE_TERM: Duration = Duration::from_secs(10 * 60);

/// A chat that holds a lease of one agent's key.
struct Held {
    grant: [u8; 16],
    model: String,
    chat: Weak<CloudConversationSource>,
}

/// The agents whose keys this device leases, and the chats that hold such a lease.
#[derive(Default)]
pub(super) struct Leases {
    tracked: Mutex<HashMap<(HubId, AgentId), Tracked>>,
}

struct Tracked {
    runner: [u8; 16],
    chats: Vec<Held>,
}

impl Leases {
    /// Starts tracking `agent` on `runner`; chats attached earlier stay.
    pub(super) fn track(&self, hub: &HubId, agent: &AgentId, runner: [u8; 16]) {
        if let Ok(mut tracked) = self.tracked.lock() {
            tracked
                .entry((hub.clone(), agent.clone()))
                .and_modify(|entry| entry.runner = runner)
                .or_insert(Tracked {
                    runner,
                    chats: Vec::new(),
                });
        }
    }

    fn attach(&self, hub: &HubId, agent: &AgentId, runner: [u8; 16], held: Held) {
        if let Ok(mut tracked) = self.tracked.lock() {
            tracked
                .entry((hub.clone(), agent.clone()))
                .or_insert(Tracked {
                    runner,
                    chats: Vec::new(),
                })
                .chats
                .push(held);
        }
    }

    /// The runner and the live chats of `agent`; dead chats are dropped on the way.
    fn live(
        &self,
        hub: &HubId,
        agent: &AgentId,
    ) -> Option<([u8; 16], Vec<(Held, Arc<CloudConversationSource>)>)> {
        let mut tracked = self.tracked.lock().ok()?;
        let entry = tracked.get_mut(&(hub.clone(), agent.clone()))?;
        entry.chats.retain(|held| {
            held.chat
                .upgrade()
                .is_some_and(|chat| nocterm_cloud_model::projection::ports::CapturedConversationSource::check_current(&*chat).is_ok())
        });
        let chats = entry
            .chats
            .iter()
            .filter_map(|held| {
                Some((
                    Held {
                        grant: held.grant,
                        model: held.model.clone(),
                        chat: held.chat.clone(),
                    },
                    held.chat.upgrade()?,
                ))
            })
            .collect();
        Some((entry.runner, chats))
    }
}

/// What a lease needs: the agent's key and the runner's verified boot.
pub(super) struct Prepared {
    key: OwnAgentKey,
    boot: TrustedProviderBoot,
}

impl Prepared {
    /// The provider the runner's configuration names for this key.
    pub(super) fn provider(&self) -> &str {
        self.key.provider()
    }

    pub(super) fn model(&self) -> &str {
        self.key.model()
    }

    /// When the verified boot ends: no lease outlives it.
    fn boot_expires(&self) -> Result<Timestamp, crate::cloud::ProviderBootError> {
        self.boot.expires()
    }

    /// Seals the key for one chat; the capsule ends with `LEASE_TERM` or the boot.
    fn seal(
        &self,
        grant: [u8; 16],
        model: &str,
        now: Timestamp,
    ) -> Result<ProviderAccess, CloudChatError> {
        let expires_ms = now
            .as_millis()
            .saturating_add(u64::try_from(LEASE_TERM.as_millis()).unwrap_or(u64::MAX));
        let sealed = self
            .boot
            .seal_lease(
                &LeaseRequest {
                    provider: self.key.provider(),
                    lease_id: grant,
                    expires_ms: i64::try_from(expires_ms).unwrap_or(i64::MAX),
                },
                self.key.expose().as_bytes(),
            )
            .map_err(error::boot)?;
        // The epoch grows with every renewal, so a newer capsule replaces the older one.
        ProviderAccess::new(
            grant,
            sealed.expires().as_millis(),
            sealed.context().to_vec(),
            vec![model.to_owned()],
            sealed.capsule().to_vec(),
            sealed.expires(),
        )
        .map_err(CloudChatError::Projection)
    }
}

/// Reads the agent's key and verifies the boot of `runner`.
pub(super) async fn prepare(
    shared: &Shared,
    account: &HubAccount,
    opened: &OpenedAccount,
    agent: &AgentId,
    runner: &[u8; 16],
) -> Result<Prepared, CloudChatError> {
    let registration = opened.runner(runner)?;
    let work = account.work().map_err(error::hub)?;
    let key = own_keys::get(&work, agent.as_str())
        .await
        .map_err(own_key)?
        .ok_or(CloudChatError::NoProviderKey)?;
    let owner = AccountProviderBoot::capture(
        opened.material(),
        registration,
        shared.budget().boot_resident()?,
        shared.collaborators().timer.clone(),
        shared.handle().clone(),
    )
    .map_err(error::boot)?;
    let boot = owner
        .fetch(shared.budget().boot()?)
        .await
        .map_err(error::boot)?;
    Ok(Prepared { key, boot })
}

fn own_key(error: CloudAgentError) -> CloudChatError {
    match error {
        CloudAgentError::Hub(error) => error::hub(error),
        other => CloudChatError::Agent(other),
    }
}

/// Hands `chat` a lease of the prepared key and tracks it for renewal. Returns how long the
/// lease lasts.
pub(super) async fn lease_chat(
    shared: &Arc<Shared>,
    prepared: &Prepared,
    hub: &HubId,
    agent: &AgentId,
    runner: [u8; 16],
    chat: &Arc<CloudConversationSource>,
) -> Result<Duration, CloudChatError> {
    let grant = nocterm_keys::random_bytes::<16>()
        .map_err(|failure| CloudChatError::Backend(nocterm_core::SharedCause::new(failure)))?;
    let model = prepared.model().to_owned();
    let remaining = send(shared, prepared, grant, &model, chat).await?;
    shared.leases().attach(
        hub,
        agent,
        runner,
        Held {
            grant,
            model,
            chat: Arc::downgrade(chat),
        },
    );
    Ok(remaining)
}

/// Seals and sends one capsule; returns the time left on it.
async fn send(
    shared: &Arc<Shared>,
    prepared: &Prepared,
    grant: [u8; 16],
    model: &str,
    chat: &Arc<CloudConversationSource>,
) -> Result<Duration, CloudChatError> {
    let now = shared.collaborators().timer.now();
    let access = prepared.seal(grant, model, now)?;
    let ends = access.expires();
    let budget = shared.clone();
    chat.grant_provider(
        access,
        Arc::new(move |bytes| {
            budget
                .budget()
                .work(bytes)
                .map_err(|_| ProjectionError::Capacity)
        }),
    )
    .await?;
    Ok(Duration::from_millis(
        ends.as_millis().saturating_sub(now.as_millis()),
    ))
}

/// Sends a fresh capsule to every live chat of `agent` and returns the shortest time left.
/// Fails with `LeaseEnded` when the agent is not tracked.
pub(super) async fn renew(
    shared: &Arc<Shared>,
    account: &HubAccount,
    hub: &HubId,
    agent: &AgentId,
) -> Result<Duration, CloudChatError> {
    let (runner, chats) = shared
        .leases()
        .live(hub, agent)
        .ok_or(CloudChatError::Agent(CloudAgentError::LeaseEnded))?;
    let opened = OpenedAccount::open(account, shared.handle(), shared.budget()).await?;
    let prepared = prepare(shared, account, &opened, agent, &runner).await?;
    let mut shortest = None::<Duration>;
    for (held, chat) in chats {
        let left = send(shared, &prepared, held.grant, &held.model, &chat).await?;
        shortest = Some(shortest.map_or(left, |old| old.min(left)));
    }
    // With no chat to refresh, the boot check above is the lease this term.
    Ok(shortest.unwrap_or(LEASE_TERM))
}

/// Verifies the key and the runner, tracks the lease and tells the service how long to wait
/// before renewing. The runner is the first own runner that is not an agent's seat.
pub(super) async fn start(
    shared: &Arc<Shared>,
    captured: &Arc<crate::devices::Captured>,
    account: &HubAccount,
    hub: &HubId,
    agent: &AgentId,
) -> Result<OwnRunnerSeat, CloudChatError> {
    let opened = OpenedAccount::open(account, shared.handle(), shared.budget()).await?;
    let seats = super::managed::seats(captured).await?;
    let runner = opened
        .runners()?
        .iter()
        .map(|row| *row.binding().runner().as_bytes())
        .find(|id| !seats.contains_key(id))
        .ok_or(CloudChatError::NotEnrolled)?;
    let prepared = prepare(shared, account, &opened, agent, &runner).await?;
    let ends = prepared.boot_expires().map_err(error::boot)?;
    shared.leases().track(hub, agent, runner);
    let now = shared.collaborators().timer.now();
    let left = Duration::from_millis(ends.as_millis().saturating_sub(now.as_millis()));
    Ok(OwnRunnerSeat {
        runner: runner.to_vec(),
        lease_ttl: left.min(LEASE_TERM),
    })
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
        assert!(leases.live(&hub, &agent).is_none());
        leases.track(&hub, &agent, [1; 16]);
        let (runner, chats) = leases.live(&hub, &agent).unwrap();
        assert_eq!((runner, chats.len()), ([1; 16], 0));
        // Another hub does not share the lease.
        assert!(
            leases
                .live(&self::hub("https://two.example"), &agent)
                .is_none()
        );
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
        assert!(matches!(
            agent_error(CloudChatError::Agent(CloudAgentError::LeaseEnded)),
            CloudAgentError::LeaseEnded
        ));
    }
}
