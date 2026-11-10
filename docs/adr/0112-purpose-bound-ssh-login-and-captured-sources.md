# 0112. Purpose-bound SSH login and captured connection sources

Status: accepted. Extends 0060, 0100, 0102, 0107 and 0110.

## Context

A remote runner must continue an authenticated connection when its authorizing
device quits, without receiving private keys or an unrestricted signing service.
A host fingerprint alone does not prove which initial SSH session is being signed.
Separately, checking an account only before spawning a transport cannot prevent a
later native worker from selecting a replacement account's relay client.

## Decision

The session domain separates immutable runtime `ConnectionOpenOptions` from the
serializable `ConnectRequest`. Options retain a narrow synchronous source guard
and optional captured relay. Unsupported producers fail closed. Native SSH stream
polls, retained SFTP/exec ports and local process admission check the original
source; pending future polls and completed reads check again. Independent producer
closure wakes native waiters. Cleanup owns cancellation, including a dropped or
timed-out initial handshake. Checks do not undo previously admitted effects.

Hub relay capture selects the actual SDK client and membership before spawning.
It binds hub, node and the complete target. Fixed store and relay consumers share
one native captured-client owner; it refreshes only the original client. Late
revocation retires only exact original membership. Erasing keys/cache additionally
requires the original open-key pointer, so late A work cannot erase B.

The SSH key domain owns bounded complete parsing of the initial host proof and
ordinary RFC4252 public-key userauth action. The device verifies the pinned host's
Ed25519 signature over the 32-byte initial session identifier, exact principal,
service `ssh-connection`, method `publickey`, signed flag, Ed25519 algorithm and
selected public key, with no trailing bytes. A purpose-specific device key owner
can sign this checked action only and exports no key or generic signing operation.

Cloud-model owns a finite device-issued consent ledger: at most 32 live/tombstoned
attempts, each at most 60 seconds. Consent binds the original account/run/runner
and epochs, target and route, selected key/credential revision, random attempt and
nonce, and time window. The device generates attempt and nonce. Its authority
collaborator checks actual captured membership and policy; descriptive bindings
or requests cannot mint consent. Exact original consent is consumed before key
loading, then source/time/revision checks repeat after waits. Failed or cancelled
signing requires fresh consent. Ledger loss requires unlock and fresh consent.

The native runner path permits only direct curve25519-sha256/Ed25519 login, using
russh's custom signer and a bounded read-only observer of initial unencrypted KEX.
The observer stops and releases its packet buffer at NEWKEYS; no shared secret or
traffic key leaves the SSH implementation. Unsupported routes and algorithms fail
explicitly. Ordinary desktop agent/file/password authentication stays separate;
it is never a runner fallback. The authorizer is released after authentication.
Live sessions and rekey retain only runner connection authority, independently
from device key or client liveness. Broken connections require fresh login consent;
transport uncertainty remains an unknown effect outcome for the host coordinator.

Program tabs inherit an immutable `SessionSource` from their original host,
including descriptive account partition and the actual stable connection guard.
They never call a destination window's credential factory. `ProgramTransport`
accepts constrained opens only for that exact source owner, while native exec and
filesystem ports keep their existing original guards. An inherited program's
credential store fails closed. A host without a session is explicitly ThisComputer.
`CredentialOriginGuard` is shared immutably; repeated observation and reconnect do
not create a different authority identity or an Arc cycle. Account source equality
therefore continues to exclude another hub's program tab from AI access.

## Consequences and limits

The observer is specific to this reviewed initial KEX combination. A future russh
callback exposing the verified initial host signature could replace it. The proof
is ordinary direct public-key authentication from an authenticated trusted runner;
it does not implement arbitrary agent forwarding, multi-hop destination chains or
OpenSSH's hostbound authentication method. Public Cloud envelope encoding and
account authentication remain with the public contract and its native ingress.

SSH exec opens a separate channel whose initial cwd is chosen by the server.
`ShellLaunch.cwd` applies to the PTY only and is not a filesystem sandbox. Native
connection guards constrain authority, not the server operating system.
