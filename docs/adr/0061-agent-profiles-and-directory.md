# 0061. Agent profiles, an injected catalog and one directory of available agents

Status: accepted

## Context

What nocterm knew about Claude, Codex and Hermes was spread across the code base.
The AI domain built the three launches from a tuple table and listed their ids
for validation. Its sandbox listed their home directories, and its usage module
parsed one agent's rate-limit metadata for every agent. The usage readers guessed
the agent from substrings of the command, the panel matched ids to pick a usage
hint, and the shared UI matched ids to pick a brand mark. Eight call sites
resolved the settings into agents themselves. Adding an agent therefore meant
editing the domain, an adapter, the shared UI and the feature, and there was no
place to attach agents that come from somewhere other than code, such as the ACP
Registry.

## Decision

`nocterm-ai::agents` describes a known agent as an `AgentProfile` with private
fields and a const builder. The profile holds the agent's id, its name and its
pinned `LaunchDefaults`. It also holds the adapter markers that recognize a
custom launch running the agent, the home-relative state the agent writes, and a
`UsageProfile`: where usage updates carry plan limits (`UsageMeta`), how the
account reports them outside the protocol (`AccountUsage`) and what to say while
none are known. Finally it holds an optional `AgentMark`. These deviations are
named by shape (`RateLimitRecord`, `OAuthAccount`, `SessionLog`), never by brand,
so a new agent whose shapes exist needs no domain change. A new shape adds one
variant here, plus a reader in `nocterm-agent-usage` when it needs native I/O.

`AgentCatalog` validates profiles at composition: ids must be valid and unique,
the command must not be empty, inherited environment names must be permitted
and state paths must be home-relative. It finds the profile of a launch by id,
then by marker. `AvailableAgents::resolve` applies the `ai` settings to the
catalog and returns typed `AgentConfigWarning`s, and every available agent
records its `AgentSource`. The domain names no concrete agent.

`nocterm-agent-providers` (domain) holds one module per built-in agent, the
brand-mark SVGs and `catalog()`. Its conformance tests run over every profile
without naming any of them. The application injects the catalog through
`AgentServices` and serves the marks through `nocterm_ui::Assets::with_marks`.

`nocterm-agent-directory` (service) owns `AgentsHandle`: the catalog and the
supplied `SettingsHandle`. It is the only place that resolves settings into
agents. The runtime, panel and settings page read agents, profiles and the
catalog from it. The handle is retained by the agent installation and passed
explicitly; it is not a global.

## Alternatives

Zed's `AgentServer` trait objects in a global store give each agent code of its
own, but our rules forbid the global, and code per agent is what this decision
removes. Keeping brand-named dialect enums in the domain would still require a
domain edit for every agent. Resolving inside the runtime would leave the panel
and settings page with their own copies of the rule.

## Consequences

Adding a built-in agent with known shapes changes only `nocterm-agent-providers`
and the user documentation. A custom agent whose command mentions a known
adapter gets that agent's profile, including its mark. The directory is where
agents from further sources, such as ACP Registry installs, will be added; that
needs its own decision about the trust boundary of downloads.
