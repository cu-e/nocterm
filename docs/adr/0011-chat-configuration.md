# 0011. A chat owns its configuration, and every shown chat connects

Supersedes two points of [0010](0010-session-lifecycle.md): restored chats
waiting for their next message, and session configuration living only in
the session.

## Context

ADR 0010 connected a chat when it was shown, but a chat restored from
history waited for its next message. Opening a saved chat therefore showed a
composer without its mode, model and effort pickers, so the model for the
first message could not be chosen there either. The chat's configuration
lived only in its session: each new session (after idle release, a failure
or a restart) took the agent's defaults, and a chat's choices were dropped
once its first session was configured. A user who switched a chat to
another model lost that choice whenever the agent slept.

Three races came from the asynchronous release of a session lease. The
runtime learns of a release a moment after the chat drops its lease, so:

- a chat that had moved on to a new connection could receive `Connected`,
  `Stopped` or `Idle` from the old one, panicking on a missing lease or
  failing its new session;
- the scheduler evicted another idle session on every tick while a released
  one was still closing.

Agents were also told to ask the user to reconnect any closed terminal,
although nocterm can reconnect a remote session itself.

## Decision

- Every chat a panel shows requests a connection when `warm_start` is on,
  restored and archived chats included. Browsing the history still starts
  nothing, and a restored queue stays paused until send-now.
- `AgentThread::config_choices` and `mode_choice` are the chat's durable
  configuration, saved in its file (`SavedChat::config`, `SavedChat::mode`).
  The composer previews the agent's last reported options with the chat's
  values over them. `configure` gives every new session the chat's values one
  option at a time before the chat is ready; once ready, values the agent
  reports become the chat's (`sync_config`). Agent-level choices in
  `agents.toml` only seed new chats.
- Runtime events for a connection carry its key (`SessionEvent::Connected`,
  `Stopped`, `Idle`); a chat ignores events for a connection it no longer
  holds.
- Admission counts connections told to yield and closes in progress as freed
  slots; a stalled close is not counted.
- A closed remote terminal is reconnected in place
  (`TerminalAccess::reconnect`) when an agent tool call needs it, or when
  `open_terminal` names a server whose background session the chat already
  opened; the call goes ahead once connected. The agent's terminal rules say
  so.

## Alternatives rejected

- Keep restored chats lazy and fetch the agent's options without a session:
  ACP reports configuration per session.
- Store only agent-level choices: two chats of one agent could not use
  different models, and a chat would follow the last change made elsewhere.
- Match runtime events by serial or by checking the lease phase: the key
  already identifies the connection, and the lease phase cannot tell an old
  connection from a new one.

## Consequences

- Opening a saved chat starts its agent. `warm_start = false` restores the
  old behaviour, and lazy-start tests opt into it.
- Saved chat files gain `config` and `mode`; older versions ignore them.
- A tool call on a dropped server may wait up to the connect or sign-in
  timeout before it answers.
