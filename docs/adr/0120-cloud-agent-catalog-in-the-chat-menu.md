# 0120. The Cloud agent catalog lives in the new-chat menu

Status: accepted

## Context

A server can host agents that run on its own pools, not on this computer. The
person needs to see them next to local agents, start one, and manage their own
personal agents, without a second place to configure.

## Decision

`nocterm-cloud-agents-model` (domain) owns the agent, pool and start types and
three narrow ports named for their consumers: `CloudAgentCatalog` (list and
watch), `CloudAgentLauncher` (start, pool trust) and `PersonalAgents` (save and
delete). `nocterm-cloud-agents` (service) owns one entity that tracks signed-in
hubs, keeps their listings, and runs a start through its phases (`Step`,
`AwaitingTrust`, `Ready`, `Failed`). `nocterm-hub-io` implements all three ports
over the hub contract; no other crate sees contract types. The app composes the
service in `src/cloud_agents_wiring.rs` and hands it to the agent and hubs-ui
crates through their installations. No global is added.

The «+» popover of the agent panel gains a Local | Cloud switch. The Cloud tab
has search, sections per hub and scope, status chips, a start progress row and
a footer. A hub window lists only its own hub; the local window lists every
signed-in hub. The tab is hidden when no visible hub offers agents, including a
hub that answers `UNIMPLEMENTED`.

Personal agents are created, edited and deleted on the Servers page, tab
"Cloud agents" (`nocterm-hubs-ui`). The agent crate cannot depend on hubs-ui, so
the menu entry dispatches the `hubs::ManageServers` action by name.

The hub is blind to definitions. A personal definition is sealed to the chosen
pool's public key; the provider key is write-only and the version is part of the
sealed context, so editing an agent means typing its key again. The first start
on a pool shows the pool's fingerprint; confirming pins it (trust on first use)
and a changed key is refused. The setup document of a runner an agent needs is
imported automatically, and a runner grant is created when the person confirms.

## Consequences

- Opening the conversation of a started Cloud agent is not part of this change;
  the start ends at `Ready`.
- The organisation is shown by id, as the listing carries no display name.
- Personal agents work offline only (on a pool); an own-runner variant is open.
- A new hub capability needs a port method here and an adapter, not a view change.
