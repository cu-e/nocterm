# 0137. Per-server CA principals and runner certificate sessions; journal v8, boot v3

Status: accepted. Supersedes the single-principal rule of
[0119](0119-agent-ssh-ca.md) (`nocterm-agent-<ca>` for every server) for chat certificates,
and the journal version rule of [0135](0135-the-runner-publishes-chat-summaries-journal-v7.md).
Extends [0123](0123-runner-control-channel.md) with boot v3. Implements the runner side of
contract ADR 0009 (per-chat SSH binding and certificates).

## Context

A Cloud chat's agent needs SSH access to some of the person's servers, for that chat only,
without a hub that could mint access and without the runner holding a long-lived key. The
contract now carries a CA-signed binding of a chat to servers (`GrantPayload::SshBinding`),
the runner's certificate request (`SshCertificateRequested`), a device's answer with an
OpenSSH user certificate (`GrantPayload::SshCertificate`), the runner's report
(`SshAccessState`) and the account CA with one principal per server
(`nocterm_keys::agents::ssh_ca::principal_for`, `nocterm-agent-<ca>-<server>`). Any unlocked
account device answers a request inside the binding; the hub only relays sealed commands and
events.

## Decision

1. **Domain.** `nocterm-cloud-model::ssh_access` owns the runner's rules. `ChatSshAccess`
   admits a newer binding epoch only (older or equal epochs of a grant id stay refused, also
   after an unbind), selects the bound servers this runner may reach from `SshReach`, refuses
   a binding without egress (`NoSourceAddress`) or without a reachable server
   (`NoReachableServer`), and otherwise asks for a certificate for the admitted servers with
   the runner's egress, valid until `min(now + 8 h, binding end)`. It accepts a certificate
   only for its open request, its own subject key, exactly the principals of the requested
   servers and a validity inside the request (at most 24 h). It asks for the next certificate
   at 80 % of the current one's life or when a request's time passed unanswered, lapses an
   expired certificate to `Pending`, and ends access with `Revoked` on unbind and `Expired` at
   the binding's end, after which the key is forgotten. `GrantClass::SshBinding`,
   `CommandBody::{SshBinding, SshCertificate}`, `CoordinatorEvent::SshAccess` and the chat
   record `JournalRecordKind::SshAccess` carry it through the coordinator; SSH grants and
   their revocation are chat-scoped and name no run.
2. **Reach is selected, never constructed.** A bound server is admitted when it is an
   administrator-configured SSH session target with the same host, port and user whose
   configured host key equals the binding's, or a managed boot target with the same host,
   port and user (the binding's host key is then pinned). Egress is `[ssh] egress` of an own
   runner (it requires `[session_targets]`) or `ssh_egress` of boot v3; at most eight IP
   addresses or CIDRs. Without egress every binding is refused.
3. **Adapter.** `nocterm-hub-io` maps the contract: it checks that a binding names this
   chat's current key epoch, derives each target's per-server principal, maps the SSH events
   to tags 14 and 15, and implements `SshCertificateReader` (`RunnerSshCertificates`) with
   `CertificateFacts::read` under the binding's CA key. The runner never parses the contract.
4. **Journal v8 and key custody.** The runner journal adds `chat_ssh_keys`: per chat the
   access parts and the seed of its Ed25519 subject key, sealed as one row by the chat's
   journal cipher; the seed is never stored in the clear and stays the same within a binding.
   A row of an older chat key epoch reads as none. `cloud-runner::CoordinatorSsh` stores the
   new row before it journals a transition's command and events, and puts the previous row
   back if journaling fails. A deleted chat keeps only the replay guard. Versions 5, 6 and 7
   are upgraded in order on open; every other version is refused.
5. **Certificate sessions.** `nocterm-ssh::SshTransport::open_certificate` signs in with a
   `CertificateLogin`: the in-memory key from the seed and the certificate for it, a direct
   route, the pinned host key and no other method (no agent, key file, vault prompt or
   password). A configured SSH target of the runner's session broker uses it when a session
   request carries no device consent and the chat holds an active certificate for that
   server (`ChatSshCredentials`); without one the request waits for a device (`NeedsUnlock`).
   Sessions already established are not closed by a revocation; expiry bounds them.
6. **Renewal sweep.** The runner's delivery sweep calls `RunCoordinator::renew_ssh`, which
   loads every chat with a row once and steps each chat whose deadline passed.
7. **Boot v3.** A managed boot document is version 2 without `ssh_egress` (no egress) or
   version 3 with one to eight entries; anything else is refused. A managed runner's
   runner.toml may not state `[ssh]`.

## Consequences

- A compromised runner reaches only servers bound to its chats, until their certificates
  expire; no hub can mint or extend access, and with no device online access lapses.
- Existing servers keep their single-principal line until a device rewrites it; until then a
  device knows the binding "needs a CA update" (the runner cannot see it).
- The contract has no journal class for the SSH row: it is sealed as an effect slot under one
  reserved slot id. A contract class would replace it with a journal format change.
- Devices cannot yet name a bound server as a session target or begin a session over the
  wire; the broker path above is ready for that protocol step. Managed boot targets are
  admitted for bindings but have no session connector, so they cannot be signed in to yet.
- Chat summaries do not list bound servers yet.
