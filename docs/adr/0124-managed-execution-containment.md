# 0124. Managed execution runs tools in nested containment behind the host broker

Status: accepted

## Context

ADR 0121 gives a managed runner a trusted host and per-lease broker tokens, and
ADR 0123 delivers those tokens per chat over a control channel. The runner's
chat engine and tools (`nocterm-cloud-runner`, `ContainedLocal`) were wired for
a personal own-runner whose provider keys are sealed commands of its devices.
A managed runner has no such keys and its tools execute untrusted agent output
inside a pooled host, so provider access and tool containment each need a rule.

## Decision

- Provider access. A managed runner's execution configuration lists no
  `[[providers]]` (`ExecutionConfig::is_brokered`). Its single provider is the
  broker, `BrokeredProviders`, a `CoordinatorProviders` implementation that
  accepts one provider (`managed`), one pricing revision (`managed-v1`) and the
  models of the boot document. Each chat's engine is built from that chat's
  token lease in `ChatTokens`, so a call carries `Authorization: Bearer nvt1_...`
  to the broker endpoint and never another chat's token.
- No token means a parked run. A chat without a live lease reports
  `NeedsProviderAccess`. ISSUE wakes the chat's runs through
  `RunCoordinator::wake_provider`, the same wake `install_provider` uses, and
  REVOKE fails the next call. `install` is unsupported and `revoke` is a no-op
  because credentials are not commands of this runner.
- Containment. Tools run in `ContainedLocal` (bubblewrap, `prlimit`, an
  unprivileged user namespace) under the instance workspace; state is not
  mounted. Containment is fail-closed: when the host cannot nest it, a managed
  runner keeps serving its controls and announces `execution_ready=false`; no
  tool ever runs uncontained. Any other startup failure stays fatal.
- A managed configuration must carry both `[execution]` and `[session_targets]`
  and no providers; the personal own-runner path is unchanged.

## Consequences

- The runner never holds a provider key. Revoking a token cuts access at the
  next call; a lease is not renewed mid-run (a lapsed lease is rebuilt from the
  held token and the 30 minute cap bounds a run).
- The host reads `execution_ready` to decide whether to schedule chats here.
- `wake_provider` is a new public method of `RunCoordinator`.

## Rejected alternatives

- Resource limits without namespaces: no filesystem or network separation.
- Changing the `CoordinatorProviders` signature to pass the chat: it would touch
  every coordinator consumer; a per-chat factory keeps the port intact.
- Degrading to uncontained execution when bubblewrap is missing: violates the
  containment invariant.
