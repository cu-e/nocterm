# 0009. A chat's composer state is kept apart from its conversation

## Context

`AgentThread` held the conversation (entries, the session lease, permissions,
tool calls, persistence) and, in the same flat struct, what the user was
composing: the draft, the attachments and images for the next prompt, the
queue of prompts waiting to be sent, its pause and edit flags, the defaults
saved while a queued prompt is edited, and the next queue id. Rules about the
input — when the queue may dispatch, what an edit restores, what a restart or
a saved chat starts from — were spread over the thread's modules as field
assignments. [0005](0005-agent-runtime-service.md) left this split open.

## Decision

- `Composer` (`crates/agent/src/thread/composer.rs`) owns the input state.
  `AgentThread` holds one as `composer` and no longer has those fields.
- The composer states its own rules as methods without GPUI: `dispatchable`
  and `take_next` (queued, not paused, not being edited), `accept` (a
  submitted queue clears the draft and advances the id, unless it replaced a
  prompt), `begin_edit`/`end_edit`/`default_attachments`, `restored` (a
  saved chat's composer waits paused) and `restarted` (what a replacement
  agent process starts from). They are unit tested on their own.
- The thread keeps what joins input to the conversation: the attachments a
  running prompt was sent with (`prompt_attachments`), grants, usage
  accounting, the debounced draft save and the history snapshot.

## Alternatives rejected

- A separate `Composer` entity observed by the panel: dispatch reads the
  composer and the session in one step, and the runtime reads both through
  `ClientState`; two entities would make every dispatch a cross-entity
  update, with ordering to reason about, for no independent lifetime.

## Consequences

- Code that reads input state names it: `thread.composer.queue`,
  `thread.composer.attachments`.
- Pausing the queue on a failure, stop or save error remains the thread's
  decision; it sets `composer.queue_paused`.
