# 0149. Agent access connects with the saved credentials, and profiles have one space

Status: accepted. Supersedes in part
[0148](0148-hub-chats-run-on-the-hub-and-use-the-agent-chat-ui.md) (decision 5: "without an
open session nothing changes"; the consequence that the replication still knows other scopes)
and [0101](0101-profiles-synchronize-through-a-replica-beside-the-local-document.md) (the scope
an object lives in).

## Context

ADR 0148 left two things behind. Turning on Agent access of a saved server in a hub window
refused when no session to the server was open, so the user had to open a tab first, although
the connection's saved credentials (in the account's vault) already suffice to connect. And the
profile replication of `nocterm-connections-model` and `nocterm-connections-store` still planned
moves between scopes, carried and copied secrets between spaces, kept placement requests, and
the sidebar still filed connections into organization sections and bound typed passwords to a
space (`CredentialSpaces`), although the hub serves only the account's own collection.

## Decision

1. **Agent access opens the session it needs.** When the switch is turned on and the port
   (`ServerAgentAccess::session_open`) reports no open session, the editor opens the saved
   connection without a tab through the workspace's session factory, exactly as the sidebar
   would (route, vault credentials, host key checks), and waits for its session. A session that
   asks the user to sign in or to unlock the vault is shown in its tab; one that does not come
   up within a minute (three after asking the user) fails, and the editor says why. Once the
   session is up the key line is written and `AddServer` is called as in ADR 0148; the session
   is then closed unless the user was shown it. Turning access off is unchanged. The sessions
   announced to the directory of live hosts (`ConnectedHosts`, `src/agent_hosts.rs`) are those
   of each hub installation's saved servers, where Agent access is offered; the connections of
   this computer are no longer announced. `AgentHosts` keeps only `access`; its unused
   `sessions` and `port` are removed.
2. **Rows show the agents' state.** In a hub window each saved server's row shows a short state
   from the hub's agent servers (matched by host, port and user): *Agents ✓*, *No access*,
   *Host key changed*, *Unreachable*, *Reconnect*. The port's `observe` re-renders the panel
   and the editor when the hub's list changes.
3. **One space.** `ReplicaScope`, `ReplicaSpace`, `ReplicaAccess`, `CredentialCopy`,
   `ProfileReplica::{carry, copy, spaces}`, placement requests and the move rules are removed;
   a remote change carries no scope, and of several changes to one profile in one feed the
   later one stands. A refusal (`ReplicaError::Forbidden`) restores the agreed version or keeps
   a never-agreed profile local, as before. `CredentialScope::within` and `CredentialSpaces`
   are removed: every connection of a window lives in the same space.

## Alternatives rejected

- **Open the connection in a tab.** It works, but leaves a tab the user did not ask for each
  time access is granted; the background session is closed again.
- **Let the hub write the key line with forwarded credentials.** Rejected in ADR 0148 for the
  same reason: the hub would see the credentials.
- **Keep the scope machinery for a future organization model.** Unused code with its own
  invariants and tests; a future model would be designed against its own protocol.

## Consequences

- Agent access works for a saved server the device has not connected to in this run, as long
  as its credentials are saved or the user signs in when asked.
- The reconciliation rules and the replication round lose their move and secret-carrying
  paths; a feed in which a profile changes twice applies its last change.
