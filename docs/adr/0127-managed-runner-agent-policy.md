# 0127. The managed runner enforces the agent definition policy

Status: accepted

## Context

Contract ADR 0004 gives an agent definition v2 a tool policy, run limits and model
prices. The private host renders the policy into the boot document, and the runner
must honour it. ADR 0123 fixes boot v2 and is not edited; this decision extends
it with one optional field. The credential broker stays the authority for spend.
The runner only narrows what its operator configuration and the prompt command
already allow.

## Decision

**Boot v2 gains an optional `policy` object.** Unknown fields remain refused, in
`policy` too:

```json
"policy": {
  "tools": { "allowed": "all" | ["tool_name", ...] },
  "approval": "never" | "ask" | "always",
  "limits": { "max_turns": n, "max_output_tokens": n,
              "max_spend_micros_per_run": n, "max_spend_micros_per_chat": n }
}
```

`tools` is required inside `policy`; `approval` defaults to `ask` and `limits` to
none, and every limit is optional. A document without `policy` behaves as before:
every tool, the existing approval rule, no extra limit. Bounds are the contract's
(`nocterm_hub_model::agents::limits`): at most `MAX_TOOL_POLICY_ENTRIES` names of
1 to `MAX_TOOL_NAME_BYTES` bytes, `max_turns` in 1..=`MAX_AGENT_TURNS`,
`max_output_tokens` in 1..=`MAX_AGENT_OUTPUT_TOKENS`, spend limits at least 1 and
the run limit not above the chat limit. Anything else fails start-up with
`boot.policy.tools` or `boot.policy.limits`. A name that is not a catalog tool is
accepted and never matches.

**Tools.** The model is offered only the allowed catalog tools
(`RigEngineFactory::with_allowed_tools`). The SDK turn policy is built from the
same list, so a call to any other tool is refused as an invalid stream before a
batch exists. An empty list offers no tool.

**Approval.** `ask` and `always` keep the coordinator's rule: every mutating call
waits for a person (the catalog's `writes` calls); the runner has no weaker
per-tool rule to keep. `never` installs a `ToolApprovalPolicy`
(`nocterm-cloud-runner`, `CoordinatorExecution::with_approval_policy`) that skips
the wait only for a target that runs in this runner's own containment (a local
connector); mutating calls on SSH targets still ask. Read-only calls never ask.

**Limits narrow only.** The model factory lowers the command's `ModelBudget`:
turns and output tokens become the smaller of the command's and the policy's, and
calls follow the turns. The runner's budget is a run total, so `max_output_tokens`
is applied as the run's output-token total, which is never more than a per-call
reading allows. A run that reaches the turn limit completes with
`MaxTurnRequests` without another model call. The policy cannot raise a limit.

**Spend.** A pricing wrapper charges each quote of a model call (its worst-case
cost) to a per-run and a per-chat total and refuses a call that would pass
`max_spend_micros_per_run` or `max_spend_micros_per_chat` with
`InferenceError::BudgetExceeded`. The totals are in memory (4096 runs and chats
each, oldest forgotten first), restart with the runner and count a quote that is
never reserved, so they can only refuse sooner than the broker would.

## Consequences

- The host must render `policy` only to runners built from this version; older
  runners refuse the unknown field.
- `approval: "never"` is meaningful only where a contained local target exists.
- Per-chat spend does not survive a runner restart; the broker still bounds it.
