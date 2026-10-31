# 0126. A device leases a personal agent's key to the own runner

Status: accepted

## Context

A personal agent that does not work while the person is offline has no pool to hold
its sealed definition ([ADR 0125](0125-cloud-agents-tail-account-keys-journal-horizon-own-runner.md)).
It runs on the person's own runner, which must call the provider with the person's key
without ever storing it. The runner already announces a signed, short-lived boot key and
installs a provider capsule from an `IssueGrant` command into its provider catalog; nothing
on the device produced such a capsule.

## Decision

**Where the key lives.** The key of an online-only agent stays in the account's encrypted
personal collection `agent_provider_keys` (object `key-<agent>`), next to the other
account-sealed data of the device. The hub lists the agent without a pool and holds no
sealed definition and no key. Deleting the agent removes the key. `OwnAgentKey` redacts its
`Debug` and zeroizes the secret.

**Producing a lease.** `TrustedProviderBoot::seal_lease` seals the key with
`seal_provider_lease` from the account key pair to the verified boot of the runner, under a
`ProviderLeaseContext` whose end is the earlier of the lease term (ten minutes) and the
boot's own expiry. The runner opens it with `RunnerBootKey::open_provider_lease`, so a
capsule is useless after the boot changes. No plaintext key outlives the sealing call.

**Carrying it.** `ConversationIntent::GrantProvider(ProviderAccess)` is an `IssueGrant`
command of one chat. `ProviderAccess` holds the sealed capsule, its canonical context, the
allowed models and an epoch that grows with every renewal; its `Debug` shows sizes only.
`CloudConversationSource::grant_provider` stages and delivers it through the ordinary
outbox against the control revision the source last observed, without a window.

**Prompting.** A plain chat with a personal agent prompts
`ModelSelection::own(provider, model)`: the provider the agent's key belongs to, with the
pricing policy `own-v1` (`OWN_PRICING`). The own runner's configuration names that
provider and pricing revision for its models; the prompt names no target, so the runner
applies its configured default destination. The chat is created on the own runner, and a
grant is sent right after the runner claims the chat.

**Renewal.** `CloudChatsLink` implements `OwnRunnerLauncher`. `start` reads the key,
verifies the runner's boot and tracks `(hub, agent)`; `renew` seals a fresh capsule for
every live chat of the agent, returns the shortest time left, and fails with `LeaseEnded`
when the agent is not tracked. The `CloudAgents` service renews at half the term and stops on
dismiss, untrack or failure; the lease then ends on the runner when its capsule expires.
Chats are held weakly, so a closed chat is no longer renewed.

**Saving.** `PersonalAgents::save` accepts a draft with "work when I'm offline" off: it
stores the key as above and lists the agent without a pool.

## Consequences

The own runner needs a provider entry whose id equals the agent's provider id and whose
models carry `pricing_revision = "own-v1"`; without one the prompt parks in
`NeedsProviderAccess`. A chat opened from history after the device went offline is not
leased until the agent is started again. The ten-minute term bounds how long a stolen
runner could use a capsule. Accepted ADRs 0113 to 0118 and 0122 are unchanged.
