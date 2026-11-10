# 0110. Local chat history captures its device or account environment

Status: accepted. Extends the context ownership and history persistence decisions.

## Context

A project or General context describes membership inside a workspace. It does not
identify the device or authenticated hub account whose Local history is visible.
A single installation can show multiple hub accounts, windows and projects.
Reading a mutable selected account when deferred work completes would give an old
chat access to a replacement account, including when the user returns to the same
persistent account identity after signing out.

## Decision

`nocterm-ai::local_environment` owns descriptive `LocalEnvironment` and `ChatScope`.
The latter pairs an environment with an independent `ContextId`. `ChatKey` includes
both coordinates and the chat ID. Its `scope()` method continues to mean
`ContextId`. Persistent identity is descriptive and grants no execution authority.

A stable account partition uses `hubs-model::source_identity::AccountScope`: the
raw hub instance label with its authenticated authority fingerprint, and the
account ID with its authenticated account-key fingerprint. A legacy hub without a
stable authenticated instance can retain Local functionality through an explicit
`LegacyConfiguredAccount` partition. It includes the retained configured `HubId`,
the actual signed-in account ID and the previously authenticated account-key
fingerprint. Such a partition is not a stable Cloud authority. Different configured
aliases of a legacy hub can consequently have different device-local histories;
address equality never manufactures authenticated hub identity.

`LocalAccountSourceFactory` captures the current native owner. Its returned
`LocalAccountSource` exposes that immutable partition and checks whether its
original authenticated owner remains current. Constructors validate shape only.
The hub adapter authenticates these facts. Token refresh can preserve the owner;
locking, sign-out, a new login, or account/device/key replacement invalidates old
captures permanently. A common notification is only a reason to check the owner.
It is not a generation or authentication fact.

`RuntimeHandle` provides bound views of one installation Runtime. Claims, readers,
catalog entries, writer queues, residency records, deferred callbacks and execution
origins retain their original full scope and source lease. Every partition shares
the installation's existing memory, catalog, session and write budgets. A hub
window can exist while its keys are locked. A separate source binding captures a
fresh handle for new actions after unlock or sign-in; existing chats are withdrawn
when stale and never retargeted. Unavailability has no substitute environment.

New chats and forks use transcript version three. Its bounded authoritative
catalog header and body explicitly contain the same environment and context.
The filesystem namespace is `scoped-v3/<environment-key>/general/` or
`scoped-v3/<environment-key>/projects/<project-id>/`. The environment key is a
versioned SHA-256 filename key over length-delimited identity parts, not an
authentication proof. Held `LocalDirectory` descendants and private atomic
publication implement isolation. Header identity is checked before full payload
reads, overwrite, deletion and sidecar use. Sidecars remain rebuildable hints.

Version one and two belong exclusively to ThisComputer. They retain their
existing bytes and paths; reading does not move or upload them. Preferred v3
records shadow earlier device records even when corrupt, so corruption cannot
reveal an older conversation under the same identity. Editing an existing record
preserves its original format and partition. New forks receive a new v3 identity.

A withdrawn chat may flush an already registered draft to its original device-local
partition under the original writer and revision fences. It cannot initiate a new
read, claim, activation, bridge registration or tool side effect. Native desktop
tool entry points check the captured account source together with the existing
workspace and session leases. Background command runners retain the source and
check it before each poll; already admitted native work is cancelled separately.

## Consequences

The same chat UUID, account ID or hub instance label can safely appear in distinct
authenticated partitions. A to B to A selection cannot revive old callbacks.
Local chats created in a hub window remain on this installation. This decision
introduces no Cloud upload, automatic migration or remote history transport.

Portable directory identities retain compatibility for old ThisComputer General
history. New v3 namespaces, including General, require held directory capabilities.
A platform lacking them reports typed unavailability instead of weakening account
partition checks.

Credential providers can additionally supply a capture factory. Terminal
construction binds its native `CredentialStore` once; reconnect retains it.
For hub accounts the application passes the captured source to the native
`HubAccount::local_credentials` boundary. That adapter binds original keys,
authenticated membership/client and cache publication inside its own worker.
An outer Future check alone cannot protect a worker that chooses its owner later.
Unavailable initial captures produce a closed store; no later account can revive
that terminal's credential authority.

The terminal capability projection declares its immutable account partition and
separate source-current observation. A chat seat filters every attachment,
terminal list and resolution by equality with the requesting chat's captured
environment and by currentness. Two simultaneously valid accounts therefore
cannot exchange terminal capabilities merely because their ContextId matches.
The provider repeats currentness with ExpectedSession before input, execution,
live command acquisition, sign-in and reconnect. Connection facts withdraw stale
sources; epoch metadata remains available for exact original cleanup.
