# 0129. The device allows a queued Cloud run to use the model

Status: accepted

## Context

The runner accepts a prompt (`SubmitPrompt`) by parking a queued run. It starts the run
only when a signed inference grant for that run installs (`CommandBody::InferenceGrant`,
verified against the issuer, the run, the model selection and the grant's budget and
deadline); until then the run waits and the chat shows "Waiting for permission". No client
issued the grant, so a managed agent's prompt reached the runner and stayed queued. The
grant is the person's permission to spend: it is the one model-use authority the device
signs, and the runner keeps narrowing it by the agent definition policy
([ADR 0127](0127-managed-runner-agent-policy.md)) and the host broker's budget
([ADR 0121](0121-managed-pools-as-a-trusted-gateway.md)).

## Decision

**Sending a prompt is the permission.** When the runner accepts the person's own prompt,
the chat issues one `IssueGrant` inference command for the run it queued, from the same
device and key that signed the prompt. The grant repeats the prompt's own bounds: the same
model selection, the same calls, turns, byte, token and spend limits, and a deadline of the
chat's inference lifetime from the moment it is issued. It never exceeds what the chat's
setup (`ConversationPromptPolicy`) configured, so the agent's per-run limits and the broker
budget stay the ceiling. Stop and grant revocation still end the run.

**An explicit Allow for every other run.** A run that was not auto-granted (queued from
another device, or the automatic grant was refused or lost) and any run in "Waiting for
permission" offers an Allow button. It sends the same grant for the displayed run, against
the displayed control revision, and is offered only while no grant for the chat is pending.
Allowing again after a grant ran out issues a new grant; task charges are never reset.

**Where it lives.**

- `ConversationAction::Allow` and `ConversationIntent::GrantInference` (domain). The intent
  carries no data: the run comes from the command coordinates, the bounds from the source's
  policy.
- `hub-io` builds `GrantPayload::Inference` in `ConversationPromptPolicy::inference_grant`,
  signs it like every command (`CommandKind::IssueGrant` with the run coordinate), and refuses
  it for a source without a prompt policy (`Unavailable`) or without a run (`WrongRun`).
- `CloudChat` (`cloud-chat`) issues the grant when a pending prompt of its own is accepted and
  exposes `ConversationView::can_allow`; the Agent panel renders the Allow button.
- The runner is unchanged.

## Consequences

- A chat that cannot prompt (history and controls only) can neither auto-grant nor offer Allow.
- A person who wants to approve spend per run needs a policy that does not auto-grant; none
  exists yet, so every managed-agent prompt authorizes its own run within the configured
  bounds.
- Two prompts accepted in one replay page allow only the displayed run; the other waits for
  Allow.
