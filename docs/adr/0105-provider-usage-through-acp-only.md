# 0105. Provider usage reaches Nocterm only through ACP

Status: accepted

Partially supersedes [0061](0061-agent-profiles-and-directory.md): its native
account-source contract, `AccountUsage` profile selection and provider readers.
Its catalog, directory, launch, home-state and brand-mark decisions remain in force.

## Context

Reading a provider's subscription token to request account limits makes Nocterm a
separate consumer of credentials intended for that provider's agent. Reading the
newest Codex session log also crosses the agent boundary and can project limits
from a different session than the connected chat. Neither capability is necessary
to run an ACP client or display reports the connected agent already supplies.

## Decision

Nocterm receives provider context usage, token totals, cost and plan limits only
through the running agent's ACP messages. `UsageProfile` retains the metadata
grammar (`UsageMeta`) and the hint shown before limits are reported. The runtime's
private `UsageLimits` owner retains reports per agent and merges each window
independently. Missing or malformed metadata neither fabricates a limit nor clears
another valid window.

There is no native account-source port, provider-usage reader adapter or account
refresh scheduler. Opening the usage card and completing a prompt use retained
projections without provider requests or provider session-file reads.

Authentication remains with the agent through its advertised ACP authentication
or terminal-setup flow. Existing environment passthrough, launch configuration and
sandbox access to the agent's own state directories remain available to the agent.
Nocterm does not extract, refresh or use subscription tokens itself. This technical
boundary does not grant provider approval to an adapter or distribution model.

## Alternatives

Retaining disconnected readers and their source contracts leaves credential
handling available for accidental reuse. Removing only the Claude reader leaves
the Codex session-file fallback outside the same boundary. Replacing either with
another private endpoint or file format preserves the coupling.

## Consequences

Context, tokens and cost remain available when the agent reports them. Plan limits
can be incomplete or unavailable, including for agents without a supported ACP
metadata shape; the UI says when no report has arrived. Supporting another report
shape requires agent-provided ACP data and a profile grammar, not a provider
credential or local log reader. No settings or saved chat migration is needed.
