# 0101. Profiles synchronize through a replica beside the local document

Status: accepted

## Context

A hub keeps a sealed copy of the user's saved connections so that several devices share them.
Profiles are edited on every device, also while the hub is unreachable. The local
`connections.toml` must keep working without a hub and stay the document the editor, the sidebar
and jump-host references read.

Writing the whole document to a remote backend (a `ConnectionRepository` or `SnapshotWriter` for
the hub) loses the edits of the device that writes second and has no offline mode. Teaching the
protocol to `ConnectionStore` would put I/O and a hub type in a service that owns checked local
admission.

## Decision

**A replica is a port beside the document, not a backend of it.** `ProfileReplica`
(`nocterm-connections-model::replica`) reads a feed of changes after a cursor, writes or
deletes one profile against an expected version, and signals that something may have changed.
A profile is one object; deleting it leaves a tombstone that is itself a version. Scope names the
space an object lives in (personal or an organization). The adapter that implements the port
speaks the hub protocol and seals the objects; the domain port knows no protocol type.

**Three states decide every profile.** The rules compare the local profile `L`, the replica's
`R` and the last state both sides agreed on `S`, which is kept whole in `ReplicaState`. A side
that equals `S` is unchanged. `connections-model::replica::rules` returns the actions for each
combination and performs none of them:

- one side changed: the other receives it (a local change is sent against `S`'s version, a
  remote change is saved through a checked write that expects exactly the local profile);
- both changed: equal results are only marked synchronized; otherwise **the remote wins** and
  the discarded local edit is reported as `Overwritten`;
- deletion against an unchanged profile propagates; deletion against an edit keeps the edit;
- no `S`: a new profile is sent or applied as new; two unrelated copies are treated as two
  edits;
- an object the client cannot decode is never applied and never deletes anything: it is
  remembered as unreadable, the local profile and its sending are left alone, and the status
  becomes "needs a newer client" until a readable version arrives.

**`ProfileReplication` is the service.** `nocterm-connections-store::ProfileReplication`
observes `ProfilePublications`, reads changes, plans with the pure rules and applies the
result. It writes local profiles only through `ConnectionStore::save_profile_checked` and
`delete_profile` and reads them only through the store's committed profiles, so authority
stamps, derived recents and the 64 KiB limits keep one owner. Applying a remote change never
sends it back: the agreed state already holds it, so the following pass reconciles to nothing.

**A pass is repeatable.** Each successful action updates the agreed state at once, and the cursor
moves only after a pass that left nothing undone. A concurrent change (`Conflict`, or a local
profile that changed between planning and writing) repeats the pass after re-reading the feed,
at most three times, and then the failure is reported. An unreachable replica is a status, not
an error: local work continues and the next local change or notification retries. A local
document that failed to load is never reconciled, so an empty copy cannot look like deleting
everything.

**The agreed state is a `Persisted<ReplicaState>`.** The application supplies its writer and
registers it with the shutdown coordinator, so a restart continues from the saved cursor without
resending anything. `attach` keeps the state and `detach` resets it: what two sides agreed on
means nothing to another account's replica, so changing the replica is a `detach` followed by an
`attach`.

## Consequences

- The local document, editor, sidebar and jump-host references are unchanged; without a replica
  the application behaves as before.
- A remote edit can discard a local edit made while offline. This is the accepted trade of a
  last-writer-wins feed without merging; the user is told through `ReplicationEvent::Overwritten`.
- A refusal of one profile (`Forbidden`, or a local size limit) is reported and skipped and
  does not hold back the others. A profile refused for writing is tried again on the next pass.
- The adapter implementing `ProfileReplica`, its codecs, the choice of account and the
  application wiring are separate changes; `MemoryReplica` in the model's `test-support`
  exercises the service without them.
