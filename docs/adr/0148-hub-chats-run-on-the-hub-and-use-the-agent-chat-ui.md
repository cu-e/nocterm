# 0148. Hub chats run on the hub and use the agent chat UI

Status: accepted. Supersedes the Cloud records
[0107](0107-cloud-run-authority-and-host-controlled-ports.md),
[0108](0108-explicit-cloud-api-engine.md),
[0109](0109-native-cloud-journal-and-effect-recovery.md),
[0112](0112-purpose-bound-ssh-login-and-captured-sources.md),
[0113](0113-account-bound-journal-catalog.md),
[0115](0115-retained-physical-terminals.md),
[0116](0116-signed-cloud-baseline-publication.md),
[0117](0117-contained-local-tool-processes.md),
[0118](0118-account-encrypted-cloud-hub-bootstrap.md),
[0119](0119-agent-ssh-ca.md),
[0120](0120-cloud-agent-catalog-in-the-chat-menu.md),
[0121](0121-managed-pools-as-a-trusted-gateway.md),
[0122](0122-cloud-chat-creation-protocol.md),
[0123](0123-runner-control-channel.md),
[0124](0124-managed-execution-containment.md),
[0126](0126-own-runner-provider-lease.md),
[0127](0127-managed-runner-agent-policy.md),
[0129](0129-the-device-allows-a-queued-run-to-use-the-model.md),
[0130](0130-cloud-agents-only-in-hub-windows.md),
[0133](0133-cloud-chats-in-the-history-column-and-seats-woken-for-prompts.md),
[0134](0134-one-composer-for-every-conversation-and-pending-prompts.md),
[0143](0143-the-runner-publishes-chat-summaries-journal-v7.md),
[0144](0144-the-device-side-runs-on-the-contract-device-core.md),
[0145](0145-per-server-ca-principals-and-runner-certificate-sessions.md),
[0146](0146-device-transport-and-resumable-commands.md) and
[0147](0147-desktop-answers-bound-chats-and-runs-prompts-on-bound-servers.md); in
[0125](0125-cloud-agents-tail-account-keys-journal-horizon-own-runner.md), the journal horizon
and the own runner (the account-key lock port stays); in
[0100](0100-the-hub-is-a-route-step.md), the relay's node. Implements the desktop side of the
contract's `nocterm.hub.v2` protocol.

## Context

Cloud agents ran in runners and pools the hub coordinated: the desktop enrolled runners, signed
journals and baselines, answered certificate requests for bound servers, sealed provider keys
to pools and presented each chat through a separate conversation model, transcript and
composer path. Organizations, groups and administrator keys shared connections and vault
records. Version 2 of the hub protocol drops all of it: the hub itself runs an account's
agents (`AgentService`, `ChatService`), keeps their providers' keys, signs in to the account's
servers with one agent key per account (`ServerService`), and keeps only personal encrypted
collections in the vault. The desktop kept about a hundred thousand lines for the old model.

## Decision

1. **A hub chat is an agent chat.** `nocterm-hub-io::HubAgentConnector` implements the neutral
   `nocterm_ai::AgentConnector` and `AgentCommands` over `ChatService`. A launch that names a
   hub (`AgentLaunch::hub`, created by `AgentLaunch::hub_agent`) is routed to it by the
   application's connector; every other launch still starts an ACP process. A new session
   creates the hub chat; a restore reads its history and replays it as the session's history
   (`SessionOpened::history`, with `AgentCapabilitiesSummary::keeps_history`); a prompt uploads
   its attachments, sends the message with a client message id and resolves when the first run
   that started after that message ends; cancel stops the run; an approval is answered with
   Allow, Allow in chat or Deny; the model option sets the chat's model. One `Watch` per open
   chat is the only stream; nothing polls. The agent panel shows the chat with the same
   `AgentThread`, transcript, composer, approvals and history as a local chat; the separate
   conversation source, Cloud transcript and freshness phases are removed.
2. **Runs from other devices are neutral turns.** A run that this session did not start
   arrives as `SessionUpdate::Turn` (started, ended). `SessionState` keeps an external turn
   apart from its own prompt, so the transcript fills in and the composer waits until the
   turn ends; no device-specific wording reaches the domain.
3. **History lists the hub's chats.** `HubChatRepository` decorates the local
   `ChatRepository`: in a hub environment it lists, renames, pins and deletes the hub's chats
   and keeps, in one private file, only which local documents started which hub chats. A hub
   chat's transcript is never written locally; loading one names the hub chat as its session,
   and the restore reads the history from the hub. The local history partition of a hub window
   depends on the signed-in account alone, so a locked vault never blocks chats.
4. **Hub agents belong to the hub's windows.** `nocterm-hub-agents` (service) keeps per tracked
   hub the agents and agent servers listed through the ports of `nocterm-hub-agents-model`
   (domain) and follows the chat list's change hints; the agent installation offers a hub's
   agents only in that hub's windows (`AvailableAgents::offered_in`), and a chat hint reloads
   that window's history. The Servers page gains an Agents tab (agents and providers) and an
   Agent access tab (the agent key with its rotation, the servers with Check and Disconnect).
5. **Agent access goes through the user's session.** The connection editor of a hub window
   offers Agent access of a saved server through a consumer port, `ServerAgentAccess`. Turning
   it on writes the agent key's line to `authorized_keys` over the session the user opened
   (the cooperative SFTP edit of ADR 0139) and only then calls `AddServer` with the host key
   this device recorded in `known_hosts`; without an open session nothing changes and the
   editor says so. Turning it off removes the line while a session is open and always removes
   the hub's record. The agent key never reaches the desktop as private material.
6. **The account keys have their own lock.** A hub's account keys follow `[hubs]
   key_lock_minutes` (zero keeps them open), independent of the local vault's auto-lock.
7. **Removed.** The runner, the Cloud runtime and its journal, the Cloud API engine, the
   Cloud conversation model and service, headless agent tools and their contained local
   processes, the purpose-bound SSH login and its proofs, the agent SSH CA, organizations, groups, administrator keys, the connection
   editor's space chooser and the relay's node. The contract's device core and Link crates are
   no longer dependencies.

## Alternatives rejected

- **Keep a separate conversation view for hub chats.** It duplicated the transcript, composer
  and approvals and diverged in behaviour; with the hub running the agent, an agent connector
  is the narrower contract.
- **Store hub transcripts locally.** A second copy would need its own invalidation and could
  disagree with the hub; the hub is the record, and a restore reads it.
- **Let the hub write `authorized_keys` with a password the desktop forwards.** The desktop
  already has an authenticated session; forwarding credentials would widen what the hub sees.

## Consequences

- A hub agent works with the hub's servers only after Agent access was turned on for them from
  a session; a server the device never connected to cannot be added from the desktop.
- Closing the window or turning the computer off does not stop a hub chat.
- The replication engine still knows scopes other than personal; the hub offers none, so
  they never appear. Removing them is separate work.
- A turned-off server whose line could not be removed (no session) keeps the line until the
  person removes it.
