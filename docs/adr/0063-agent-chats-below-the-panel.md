# 0063. Agent chats live in a service crate below the panel

Status: accepted

## Context

`nocterm-agent` held both the chat panel's views and everything a chat does
without them: the `AgentThread` entity, its session driver, tool runtime,
approvals, queue, saved document and archive hydration. The chats reached the
window through the workspace entity, the panel entity (to deliver opened and
forked archives) and an opener closure typed with the workspace. Chat membership
and lifecycle (new, restart, fork, restore, removal, resident history) were
panel methods interleaved with view resets. Nothing in the chats needed the
component library, yet a change to them rebuilt and retested the whole panel,
and the chats could not be reused by another surface.

## Decision

The `nocterm-agent-chat` service crate owns chats without views:

- `thread::AgentThread` and its private owners, unchanged in behaviour;
- `roster::ChatRoster`: a window's chats, the open chat, creating chats (new,
  restarted, forked, restored, archived), admitting a chat's document before it
  is removed, and the resident history budget;
- `chat_deps::ChatDeps`: what every chat receives (runtime, agent directory,
  settings, chat metadata, seat, sign-in terminal);
- two ports the window shell implements: `seat::ChatWorkspace` (read terminals
  and connections, observe membership, open, show and close the chat's
  background sessions) and `sign_in::SignInTerminal` (run an agent's sign-in
  command in a terminal);
- `TerminalAuthError`, re-exported by `nocterm-agent`.

Chats name terminals through `nocterm-workspace-ports`
([ADR 0062](0062-terminal-contracts-below-the-workspace.md)) and GPUI core.
Archive delivery is a callback the requester supplies, so a chat refers to no
panel. `nocterm-agent` keeps the panel, its presentation owners, `ChatHost`
(view collaborators plus the panel's `ChatDeps`), `WorkspaceSeat` and
`WorkspaceSignIn`.

Test-only inspection and driving helpers of the chat crate are compiled for
dependents behind its `test-support` feature.

## Alternatives

A separate `agent-tools` crate for the tool runtime was deferred: the tool
runtime is driven only by the thread and shares its session authority, so a
boundary there would add a port with one caller. Renaming `nocterm-agent` to
`nocterm-agent-ui` was deferred because the architecture debt inventory is keyed
by crate name and may only shrink.

## Consequences

The chats build and test without the component library. The panel uses chats
through their public API, which is wider than a view needs; narrowing it to
projections is follow-up work. Another surface can host chats by implementing
`ChatWorkspace` and `SignInTerminal`.
