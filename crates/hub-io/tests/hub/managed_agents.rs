//! Starting a managed agent the way the Cloud tab does, against a hub with an agent host.
//!
//! They need `NOCTERM_HUB_AGENT`, the id of an agent whose pool has an online host (the demo
//! agent of the server's `agents-demo` profile). Without `NOCTERM_REQUIRE_HUB` they pass
//! vacuously when it is missing; with it, a missing variable fails the run.
//!
//! Two tests are opt-in. `NOCTERM_HUB_AGENT_UNPINNED` names an agent the account has not started
//! yet and whose start the host refuses because it holds no pin for the account's key (the
//! host's `stated_account_keys` is `none` and `pins_file` has no `accounts` entry). Setting
//! `NOCTERM_HUB_AGENT_CHAT` (any value) opens a chat on the demo agent, which needs a runner
//! that can run tools, sends a prompt and expects the model's reply; `NOCTERM_E2E_RUNNER_IMAGE`
//! additionally checks `docker ps` for it. Setting `NOCTERM_HUB_AGENT_WAKE` opens a chat on the
//! account's existing seat of the agent the way a row of the Cloud tab does, without removing
//! it: a seat that slept wakes, and its runner must claim the chat.
use super::{hub, links, sign_in};
use futures::executor::block_on;
use nocterm_cloud_agents_model::{
    AgentId, CloudAgentError, CloudAgentLauncher as _, StartOutcome, StartStep, StepSink,
};
use nocterm_cloud_agents_model::{AgentSeats as _, ChatHost, CloudChatLauncher as _};
use nocterm_hub_io::{ChatServices, CloudAgentsLink, CloudChatsLink};
use nocterm_hub_model::agents::ProvisionFailure;
use std::sync::{Arc, Mutex};

fn agent(variable: &str) -> Option<AgentId> {
    let Ok(id) = std::env::var(variable) else {
        assert!(
            std::env::var_os("NOCTERM_REQUIRE_HUB").is_none(),
            "NOCTERM_REQUIRE_HUB is set but {variable} is not"
        );
        return None;
    };
    Some(AgentId::new(id).expect(variable))
}

fn recording() -> (StepSink, Arc<Mutex<Vec<&'static str>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let seen = seen.clone();
        Arc::new(move |step: StartStep| {
            seen.lock().unwrap().push(match step {
                StartStep::ReservingSeat => "reserving",
                StartStep::TrustingPool(_) => "trusting",
                StartStep::Registering => "registering",
                StartStep::Ready => "ready",
            });
        })
    };
    (sink, seen)
}

/// Starts the seat, asking again while the hub is still deleting the one just removed: the hub
/// names that as a transient refusal, not as a failure.
fn start_when_deleted(
    launcher: &CloudAgentsLink,
    hub: &nocterm_hubs_model::HubId,
    agent: &AgentId,
    steps: &StepSink,
) -> StartOutcome {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match block_on(launcher.start(hub, agent, steps.clone())) {
            Err(CloudAgentError::SeatBeingDeleted) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            other => return other.unwrap(),
        }
    }
}

#[test]
fn a_person_starts_the_demo_agent_after_comparing_the_pool_fingerprint() {
    let (Some(hub), Some(agent)) = (hub(), agent("NOCTERM_HUB_AGENT")) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    sign_in(&links, &hub).unwrap();
    let launcher = CloudAgentsLink::new(links.clone(), dir.path().join("pins"));
    let (steps, _) = recording();

    let first = block_on(launcher.start(&hub.id(), &agent, steps.clone())).unwrap();
    // The first start on this device offers the pool; the test is the person who compares the
    // fingerprint with the one the hub operator printed and accepts it.
    let StartOutcome::NeedsPoolTrust(offer) = first else {
        panic!("a fresh device must be asked to trust the pool, got {first:?}");
    };
    assert!(!offer.fingerprint.is_empty());
    block_on(launcher.pin_pool(&hub.id(), &offer)).unwrap();

    let second = block_on(launcher.start(&hub.id(), &agent, steps.clone())).unwrap();
    let StartOutcome::Ready(started) = second else {
        panic!("the pinned pool must lead to a ready seat, got {second:?}");
    };
    assert_eq!(started.agent, agent);
    assert!(
        !started.runner.is_empty(),
        "the runner enrolled with the hub"
    );

    // Starting a ready seat again is idempotent: the same runner comes back.
    let again = block_on(launcher.start(&hub.id(), &agent, steps)).unwrap();
    assert!(matches!(again, StartOutcome::Ready(ref seat) if seat.runner == started.runner));
}

#[test]
fn a_host_refusing_an_unpinned_owner_key_tells_the_operator_what_to_pin() {
    let (Some(hub), Ok(unpinned)) = (hub(), std::env::var("NOCTERM_HUB_AGENT_UNPINNED")) else {
        return;
    };
    let agent = AgentId::new(unpinned).expect("NOCTERM_HUB_AGENT_UNPINNED");
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    sign_in(&links, &hub).unwrap();
    let launcher = CloudAgentsLink::new(links.clone(), dir.path().join("pins"));
    let (steps, _) = recording();
    let first = block_on(launcher.start(&hub.id(), &agent, steps.clone())).unwrap();
    let StartOutcome::NeedsPoolTrust(offer) = first else {
        panic!("a fresh device must be asked to trust the pool, got {first:?}");
    };
    block_on(launcher.pin_pool(&hub.id(), &offer)).unwrap();

    let error = block_on(launcher.start(&hub.id(), &agent, steps)).unwrap_err();
    let CloudAgentError::ProvisioningFailed { reason, .. } = &error else {
        panic!("expected a provisioning failure, got {error:?}");
    };
    assert_eq!(
        reason.as_deref(),
        Some(ProvisionFailure::AccountUnpinned.describe())
    );
    let text = error.to_string();
    assert!(
        text.contains(ProvisionFailure::AccountUnpinned.describe()),
        "{text}"
    );
}

#[test]
fn a_started_agent_opens_a_chat_and_its_runner_container_runs() {
    let (Some(hub), Some(agent)) = (hub(), agent("NOCTERM_HUB_AGENT")) else {
        return;
    };
    if std::env::var_os("NOCTERM_HUB_AGENT_CHAT").is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    sign_in(&links, &hub).unwrap();
    let launcher = CloudAgentsLink::new(links.clone(), dir.path().join("pins"));
    let (steps, _) = recording();
    // A fresh seat every run: a runner initialized by an older build keeps its old state. The
    // start right after the removal is what a person does, and it waits for the deletion.
    block_on(launcher.remove(&hub.id(), &agent)).unwrap();
    let mut outcome = start_when_deleted(&launcher, &hub.id(), &agent, &steps);
    if let StartOutcome::NeedsPoolTrust(offer) = outcome {
        block_on(launcher.pin_pool(&hub.id(), &offer)).unwrap();
        outcome = start_when_deleted(&launcher, &hub.id(), &agent, &steps);
    }
    assert!(matches!(outcome, StartOutcome::Ready(_)), "{outcome:?}");

    // Opening a chat is what takes the lease: the host starts the runner for it.
    let services = ChatServices::native(&links, dir.path().join("chats")).unwrap();
    let chats = CloudChatsLink::new(links.clone(), services).unwrap();
    let model = std::env::var("NOCTERM_HUB_AGENT_MODEL").unwrap_or_else(|_| "qwen2.5:0.5b".into());
    let opened = block_on(chats.create(&hub.id(), ChatHost::Agent { agent, model })).unwrap();
    assert!(opened.conversation.can_prompt());

    // The model answers through the host's broker, which holds the only provider credential.
    let reply = super::chat_exchange::reply_to(
        &opened.conversation,
        "Reply with one short sentence.",
        std::time::Duration::from_secs(60),
    );
    assert!(!reply.trim().is_empty(), "the model answered nothing");

    if let Ok(image) = std::env::var("NOCTERM_E2E_RUNNER_IMAGE") {
        let listed = std::process::Command::new("docker")
            .args([
                "ps",
                "--filter",
                &format!("ancestor={image}"),
                "--format",
                "{{.Names}}",
            ])
            .output()
            .unwrap();
        let names = String::from_utf8_lossy(&listed.stdout);
        assert!(
            !names.trim().is_empty(),
            "no runner container of {image} is running"
        );
    }
}

#[test]
fn a_sleeping_seat_wakes_for_a_new_chat_and_its_runner_claims_it() {
    let (Some(hub), Some(agent)) = (hub(), agent("NOCTERM_HUB_AGENT")) else {
        return;
    };
    if std::env::var_os("NOCTERM_HUB_AGENT_WAKE").is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    sign_in(&links, &hub).unwrap();
    let services = ChatServices::native(&links, dir.path().join("chats")).unwrap();
    let chats = CloudChatsLink::new(links.clone(), services).unwrap();
    let model = std::env::var("NOCTERM_HUB_AGENT_MODEL").unwrap_or_else(|_| "qwen2.5:0.5b".into());
    let opened = block_on(chats.create(&hub.id(), ChatHost::Agent { agent, model }))
        .unwrap_or_else(|error| panic!("{}", nocterm_core::error::chain(&error)));
    assert!(opened.conversation.can_prompt());
}
