# 0119. A personal SSH CA lets cloud agents reach the user's servers

Status: accepted

Supplements [0112](0112-purpose-bound-ssh-login-and-captured-sources.md).

## Context

A cloud agent runs commands on the user's own servers. Consent-based logins
(0112) require an interactive user and a captured source, so an unattended
runner cannot use them, and sharing the user's private keys with a runner or a
hub is not acceptable. The user needs a revocable, short-lived and inspectable
grant that no hub can mint.

## Decision

- **A personal user CA.** Each account owns one Ed25519 CA. Its seed lives in
  the account's end-to-end encrypted `agent_ssh_trust` collection (object
  `ca`), never in a form a hub reads. `nocterm-ssh-keys::ca::UserCa` signs
  OpenSSH user certificates from a `CertificateSpec`: principal
  `nocterm-agent-<hex of the CA public key digest>`, at most 24 hours, a
  mandatory `source-address`, only the `permit-pty` extension (no port, agent
  or X11 forwarding, no user rc). A pty is kept because the agent's terminal
  tool needs one; every other capability stays off.
- **Servers opt in.** A server trusts the CA only when its
  `~/.ssh/authorized_keys` holds a `cert-authority` line restricted to the
  principal. The desktop edits that file through the SFTP of a session the user
  already opened (`nocterm-ssh::authorized_keys`): the edit is atomic, compared
  with the original before it is swapped in, and keeps every unrelated line.
  The user may instead copy the line and add it by hand.
- **A registry the hub cannot read.** One object per server
  (`SshTrustRecord`: connection, user, host, state, who added it and when it
  was last verified) lives in the same personal collection and syncs through
  the hub as ciphertext. States: trusted, removal pending, removed, drifted.
- **Denial survives an unreachable server.** Denying persists *removal
  pending* first, then edits the host. If the host cannot be reached the state
  stays pending and the next normal connection finishes the removal. Each fresh
  connection reconciles the registry with the file: a vanished line is marked
  *drifted*, a pending removal completes.
- **Reissuing the CA** replaces the seed (the old public key is retained for
  recognising old lines), moves the servers that can be reached now to the new
  CA and leaves the rest to drift until their next connection.
- **Certificate login.** `nocterm-ssh` gains `Auth::VaultCertificate`: the
  vault key and the certificate for it. The key is requested through the driver
  as a vault key is; a certificate that is not a user certificate or does not
  certify that very key is never sent. Consent (0112) remains the fallback when
  no certificate is available.
- **Placement.** The contract types live behind `nocterm-hub-io` (ADR 0096), so
  contract-typed logic (`agent_access`) is in `nocterm-hub-io`; the neutral ports
  `AgentHostTrust` and `AgentGrants` are in `nocterm-hubs-model`, held by
  `nocterm-hubs` and shown by the Agent access (SSH) tab of `nocterm-hubs-ui`.
  `nocterm-connections` announces every freshly connected session through
  `HostObserver`, which the application turns into the reconciliation.
- **Chat grants.** Active chat accesses are listed and revoked through the hub
  client; revocation does not depend on any server being reachable.

## Consequences

- A compromised hub can neither read the CA seed nor mint a certificate.
- A lost laptop is handled by revoking the device and, if needed, reissuing the
  CA; a server that was offline keeps trusting the old CA until the next
  connection, which the tab shows as pending or drifted.
- Pinning the keys of runner pools and the account-key lock stay outside this
  decision: the first needs the pin store of the Cloud agents tab, the second a
  narrow account-key port that does not exist yet.
