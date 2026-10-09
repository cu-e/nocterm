# 0010. A chat's session is a pure state machine, connected when shown

## Context

A chat's agent session was described by flags on `AgentThread` (generating,
cancelling, failed, opening, signing in) changed from async callbacks across
its modules. Combinations such as "generating without a session" could be
built, a late result from a replaced session could change the current one,
and the recovery rules after a failed resume, load or fork lived in those
callbacks. Three user-visible faults followed:

- The agent process started only with the first message, so the model and
  effort for that message could not be chosen: the agent had not reported
  its options yet.
- A forked chat whose fork request failed stayed failed; sending again did
  not recover it until the application was reloaded.
- An idle session was released after a short timeout even while its chat was
  on screen, so the next message paid for a cold start.

Zed, VS Code and Cursor connect an agent when its chat is opened, show the
agent's models before the first message, cache the last reported model list
for the time before the agent answers, and treat a lost remote session as a
reason to start a new one with the local transcript.

## Decision

- `SessionState` (`crates/ai/src/session.rs`) owns the session phase:
  Detached, Queued, Opening, Ready, Prompting, Cancelling, SignInRequired,
  SigningIn, Failed. Every change goes through a transition method that
  decides and returns what the chat must do; the chat performs the side
  effects. Each async request carries a `Ticket` (session epoch, turn); a
  result whose ticket is no longer current is dropped.
- `restore_fallback` decides what follows a failed resume, load or fork:
  sign in on authentication errors, start fresh when the agent no longer has
  the session, retry once, then start fresh for a fork or fail for the chat's
  own session. A failed chat reconnects on the next message;
  `may_resume` starts fresh after repeated failures since the last completed
  turn.
- Warm start (`[ai.sessions] warm_start`, on by default): a chat a panel shows
  requests a connection at once. A chat hidden while still queued withdraws
  its request unless it has messages to send. A chat restored from history
  waits with its queue paused and connects on its next message, so starting
  the application or browsing history launches no agent.
- Session configuration is stored without ACP types
  (`crates/ai/src/session_config.rs`): `agents.toml` keeps the options each
  agent last reported and the model and reasoning effort last chosen. A new
  chat previews them; choices made before the session opens are applied one
  option at a time before the chat becomes ready, because a model decides the
  efforts it offers. A value the agent no longer offers is skipped; a rejected
  one leaves the agent's choice and a status line. Modes are never carried
  over.
- Admission (`crates/agent-runtime/src/admission.rs`, `releases`) keeps the
  connection of a shown chat beyond `idle_timeout_secs` and `max_idle`; it
  yields only when a waiting chat needs its slot and no hidden idle
  connection can. Defaults: `max_live = 4`, `max_idle = 3`,
  `idle_timeout_secs = 1800`.

## Alternatives rejected

- Start the agent when the panel opens, without a chat: it would launch
  processes for users who only read history, and the options belong to a
  session, not to the agent.
- Hold choices until the first prompt and send them with it: ACP has no
  per-prompt configuration, and a model change can invalidate the effort
  chosen with it.
- Keep every session alive: each one is an agent process with its MCP
  bridge; the shown chat is the one the user is most likely to write in.

## Consequences

- Creating a chat starts its agent; `warm_start = false` restores connecting
  on the first message, and the existing lazy-launch tests opt into it.
- A saved chat does not store its agent's options, so they appear only once
  its session reopens.
- The options shown before a session opens may be stale until the agent
  answers; the agent's reply replaces them.
- `agents.toml` gains `options` and `choices`; older versions reject the file
  (`deny_unknown_fields`) and lose its favorites on their next save.
- Saved chat history still stores ACP types; a versioned storage format
  independent of the protocol is a separate step.
