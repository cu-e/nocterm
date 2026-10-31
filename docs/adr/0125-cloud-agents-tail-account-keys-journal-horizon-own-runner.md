# 0125. Account-key lock port, journal v6 horizon and own-runner placement

Status: accepted

## Context

Three open ends of the Cloud agents work needed one decision each: the Agent
access tab could not unlock a locked account key, the outbox retry horizon
lived only in memory and so restarted with every process, and a personal agent
that does not work while the person is offline had no defined place to run.

## Decision

**Account keys.** `nocterm-hubs-model::AccountKeyLock` is a narrow port named for
the Agent access tab: report whether the keys of a hub are open, unlock them
with a typed password, lock them. The hub-io adapter wraps the account slot of
the vault and derives the key with the cached keyset method (login password or
vault passphrase). The password is a `Secret`, never stored by the view. The
vault idle timer is not restarted by an unlock from this tab; the keys stay
open until locked or the process ends.

**Journal v6.** The runner journal stores `retry_until` (milliseconds since the
epoch) per outbox entry as a plain, unsealed column: it is scheduling data, not
content. `SCHEMA_VERSION` is 6. A v5 journal is upgraded in one transaction on
open: the column is added, and each unacknowledged entry receives
`now + OUTBOX_RETRY_WINDOW_MS` (seven days). Acknowledged entries keep 0. Any
other version is still refused. An entry appended after the upgrade receives
`append time + window`. The persisted horizon is not rewritten when it has
passed; the in-process `RetryHorizons` ledger is only a fallback for hubs that
do not report a persisted horizon.

**Own-runner placement.** `nocterm-cloud-agents-model::Placement` decides where
an agent runs. A personal agent no pool holds (and a draft with "work when I'm
offline" off) runs on the person's own runner; all other agents run on a pool.
The `OwnRunnerLauncher` port starts the own runner and leases the provider
credential on this device; the `CloudAgents` service renews the lease at half
its term in the background while the start is listed, and stops when the start
is dismissed, its hub is untracked or a renewal fails. The routing and renewal
are domain/service behaviour; the hub-io adapter and the chat that uses the
runner belong to the chat-opening path and answer `OnlineOnlyUnavailable` until
that path lands.

## Consequences

Old journals keep their entries with a fresh seven-day horizon; none expires
early. A person who turns "work offline" off sees a typed unavailable message,
not a pool start with a key the person did not want to seal. Accepted ADRs 0119
to 0121 are unchanged.
