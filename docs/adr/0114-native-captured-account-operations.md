# Native captured account operations

Status: accepted

## Context

A configured hub has a current signed-in client and a general vault window. A queued
operation, an idle timeout or a displayed confirmation can outlive the login that
created it. Looking up the current client or open keys after an await can apply an
old operation to another account, including a new login of the same account.
Descriptive account ids and grantor fingerprints do not establish that authority.

Unlock and recovery must remain available while the account keys are closed. A
window therefore cannot use an open-key capture as its entire lifetime boundary.

## Decision

`nocterm-hub-io` separates authenticated native membership from opened account keys.
The Pool owns admission under its existing client mutex. A replacement login binds
the Slot to the exact new native membership and retires its old open keys before
publishing that membership. Normal refresh of the same local login retains it.

The Slot owns a nonpersistent lifecycle receipt. Explicit lock, key installation,
recovery and erasure rotate the receipt. Notifications and ordinary encrypted cache
updates do not. Key publication requires both the original native membership and
its exact Slot receipt while native admission is held. The lock order is clients,
then credential file or Slot/cache, then dependent timer or presentation state;
no admission closure awaits a network operation.

`AccountOperations` captures the native source and the shared store adapter before
enqueue. Vault, profile replica and captured terminal credential workers reuse the
same store RPC implementation and vault ledger. They check their original source
on worker admission, after awaits and before returning a completed result to its
caller. Cache, replica spaces and idle timer publication remain conditional under
native admission and the Slot lock. Bound replica facades never select another
login; app composition captures a fresh facade when attaching the current source.
The general vault instead captures membership for each explicit unlock or recovery,
so closed keys do not disable those operations.

Reconnect, password/OIDC login and sign-out retain their original native selection
and private credential-file origin before enqueue. Login publication reuses the
local login identity and bound session store. A produced membership is passed to
key work before directory lookup; returned directory data must agree with the
actual authenticated subject. Delayed retirement is conditional on the original
local file login and cannot erase a replacement login or its encrypted cache.
`SignedIn` carries a `LoginReceipt` through settings and recovery awaits; failed
settings publication conditionally discards only that original login. A delayed
consumer never calls unconditional configured-hub removal to clean up a login.
Already admitted remote I/O may finish for its original device; native guards do
not invent a remote session-family guarantee absent from the server contract.

`nocterm-hubs-model` provides opaque administrator and recovery offers. The native
adapter supplies their authority; constructing display data does not authenticate
it. An administrator offer binds the original recipient and open keys as well as
the displayed grantor key. A recovery acknowledgement binds the original recipient
and exact pending recovery generation; an earlier displayed key cannot acknowledge
a later reissue. These distinct capabilities pass unchanged through `nocterm-hubs`
and `nocterm-hubs-ui` to the existing confirmation actions. Stale actions return
typed errors. No global lookup or additional approval step is introduced.

## Consequences

Same-account re-login and A → B → A cannot revive an old worker or confirmation.
Explicit vault lock cancels an in-flight unlock, while ordinary refresh preserves
valid captures. Long-lived facades and displayed offers retain weak key authority;
only admitted bounded work retains open keys. Source identity constructors remain
descriptive, and older configured hubs do not gain stable Cloud authority through
these native lifetime checks.
