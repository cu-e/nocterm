# 0147. The desktop binds chats to servers and answers their certificate requests; prompts run on bound servers; journal v9

Status: accepted. Supersedes, in [0145](0145-per-server-ca-principals-and-runner-certificate-sessions.md),
the SSH row sealed as an effect slot (decision 4 and its consequence), the rule that a managed
boot target pins the binding's host key (decision 2), the journal version rule of decision 4,
and the consequences that devices cannot name a bound server, that managed boot targets cannot
be signed in to and that summaries list no bound servers. Supersedes, in
[0119](0119-agent-ssh-ca.md), the desktop's own CA signer and CA document (`nocterm-ssh-keys::ca`,
the single principal of every server's line) and the copy of one shared CA line.
Implements the desktop side of contract ADR 0009 (per-chat SSH binding and certificates) and
contract ADR 0011 (bound servers as session targets, the SSH journal class, boot v3 targets and
summary servers).

## Context

With ADR 0145 the runner admits a person's CA-signed binding of a chat to servers, asks for
short certificates and signs in with them, but no desktop device signed bindings or answered
requests, the runner sealed its SSH row as an effect, a device could not ask for a prompt to run
on a bound server, a managed runner could not sign in to its boot targets and summaries listed
no servers. Contract ADR 0011 added what was missing: `JournalClass::SshAccess`, a bound server's
`TargetAccess` derived from the binding, the boot v3 SSH section with optional host keys and
session settings, and `summary::bound_servers`. The desktop still kept its own CA document
layout and certificate signer, and every trusted server held the shared principal.

## Decision

1. **The contract CA.** `nocterm-hub-io::agent_access` uses `nocterm_keys::agents::ssh_ca`
   (`UserCa`, `CaDocument`, `principal_for`) and the per-server line (`CaLine::for_server`,
   `server_line`, trust record schema 2). Allowing a server, or moving it to a new CA, writes the
   line of that server's own principal and records the per-server form. A trusted server that
   still holds the shared principal is shown as "needs a CA update" on the Agent access page and
   in a chat's server choices; its next connection's reconcile rewrites the line (other lines are
   kept) and records the per-server form; a failed edit keeps the flag for the next connection.
   "Copy line" copies one server's own line. The desktop's signer moves to
   `nocterm-ssh-keys::testing::ca`, used only by the SSH tests.
2. **Binding from the desktop.** The `CloudConversation` port gains `servers()`,
   `bind_servers(servers, window)` (a window of at most thirty days, seven by default in the
   panel) and `unbind_servers()`, the transcript a `ServerBinding` (each bound server with where
   it stands and whether a prompt may name it) and a prompt its `server`. hub-io runs the core's
   `BindServers` and unbind; the servers it offers are the trusted servers of the account's
   registry, and the port of each is the one this device last reached the saved connection on
   (`AgentHosts::port`). A server whose port this device does not know yet cannot be bound
   (`ServerRefusal::UnknownPort`): a port is never guessed.
3. **Answering while unlocked.** A chat that hub-io reads answers every certificate request the
   core reports (`ChatUpdate::SshRequest`) with the core's `SshAnswer`; the core certifies only a
   request of the verified history inside a binding the account's current CA signed, not
   revoked, refused or expired, for servers still trusted. `nocterm-cloud-agents::ssh_renewal`
   sweeps each tracked hub through the narrow `ChatSshCertifier` port every minute; the hub-io
   implementation keeps every chat whose summary lists bound servers read (and lets the others
   go), so requests are answered while the person does not look at the chat. With the account
   locked or signed out the reading ends and requests wait for a device, by design.
4. **The panel.** A Cloud chat in the agent panel has a Servers bar (`ConversationServers`, an
   optional capability of a `ConversationSource`): the bound servers with their state (waiting
   for a device, active until, not reachable by this agent, refused, expired, revoked), the
   server the next prompts run on (the default destination or a selectable bound server), a form
   to bind CA-trusted servers for 1, 7, 14 or 30 days, and Unbind. History rows name the servers
   a chat is bound to.
5. **Runner: bound servers as session targets.** hub-io derives each bound server's session
   access (`SshBinding::target_access`) when it admits a binding and keeps it with the
   `BoundTarget` (and in the offer's native form). A run whose destination is no configured
   target selects the admitted server of the chat's current binding with exactly that access
   (`ChatSshAccess::select`, `ChatSshCredentials::bound`), then a connector with that server's
   host, port, user and host key: a configured SSH target or a managed boot target. It signs in
   only with the chat's certificate and waits for a device without one; a newer binding epoch or
   another CA selects nothing.
6. **Boot v3 is the contract's.** The runner reads the SSH section with
   `hub_model::agents::boot_ssh` (re-exported by hub-io, ADR 0096): `check_boot_ssh` refuses
   anything else with a typed `BootSshError`; a boot target is admitted for a binding only with a
   host key equal to the binding's (one without a host key is listed but never admitted), and a
   keyed boot target gets a session connector from its host key and session settings.
7. **Journal v9.** The SSH row is sealed under `JournalClass::SshAccess` with the slot id
   `SSH_ACCESS_SLOT`. An upgrade holds no chat keys and a row never opens under another class, so
   the v8 to v9 upgrade deletes every v8 SSH row (none left a development machine); chats and
   their records are kept and a device binds such a chat again. The same step rebuilds a v7
   summary table with the contract's sealed summary bound (6286 bytes). Versions 5 to 8 are
   upgraded in order.
8. **Summaries list bound servers.** The runner's summary facts take the binding and access its
   `CoordinatorSsh` holds at each sweep (a change is a new revision); hub-io derives the servers
   with `summary::bound_servers` at the draft's time. A runner that states egress sets
   `SUMMARY_FEATURE_SSH_BINDING`.

## Consequences

- A person binds a Cloud chat to servers from the desktop and picks the server its prompts run
  on; any unlocked desktop of the account keeps the runner's certificates fresh, and nothing
  else (no hub, no runner) can mint them.
- A compromised runner reaches only servers bound to its chats and only until their certificates
  end; servers that still hold the shared principal refuse chat certificates until they are
  connected to once.
- A desktop that restarted knows no server ports until it connects to the servers again, so a
  new binding needs one connection per server after each start.
- A chat bound on a v8 runner must be bound again.
- Established sessions are still not closed by a revocation; expiry bounds them.
