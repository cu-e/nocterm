# 0107. Cloud run authority and effects have pure owners and host-controlled ports

Status: accepted

## Context

A Cloud run outlives a visible transcript. Hub/account identity must remain distinct
from a window's current source, and a provider's tool-call identifier cannot grant
permission to mutate a remote target. Losing a provider response, a worker or a
connection must not silently replay a possibly completed command. Provider history
also cannot serve as the durable command ledger or as a journal write acknowledgement.

## Decision

`nocterm-hubs-model::source_identity` owns descriptive captured hub/account source
values shared by AI, storage and Cloud domains. A source includes the supplied hub
instance id with its authenticated authority fingerprint, and the account id with
its expected account-key fingerprint. Checked namespace values preserve literal
UTF-8 bytes and enforce a finite length. These constructors neither authenticate
an untrusted directory entry nor derive canonical hub wire namespaces. Display
addresses, profile aliases and local workspace ids never substitute for the source.

`nocterm-cloud-model` owns run, grant, effect and bounded admission policy without
I/O, SDKs, protocol types, clock reads, entropy or GPUI. Run authority retains the
original source, runner id and key epoch, chat, run and run epoch. Execution also
binds the exact target, selected SSH authentication key and credential revision,
session id and epoch, grant id and epoch. A grant records its original command,
issuer credentials and control revision. Its deadline and operation count are
finite; rollback of supplied time permanently fences the grant. Read capability
excludes execution, terminal input, cancellation and connection creation.

The pure run owner separates prepared intent, committed intent, committed dispatch
and committed outcome. Only the dispatch transition returns an effect description.
The injected executor still validates the exact grant, session, writer and argument
digest immediately before I/O. An uncertain mutation retains its original intent
and blocks fresh mutations, including a new provider call id and a resumed run
epoch. Reads may proceed, but only a durable explicit original-intent reconciliation
clears the fence. Replay describes interruption and never resubmits an effect. The
host must also retain unresolved target effects across different run ids; creating
a new pure run owner does not reconcile an earlier mutation.

The host allocates stable step and intent identities. `AgentEngine` receives only
explicit prompt/tool-result inputs, reports bounded text/thought/optional usage,
and returns tool calls or completion after verified provider completion. Provider
call ids remain descriptive; host tool slots correlate answers through restore.
The derived checkpoint is opaque, bounded, versioned and bound to its original run
and journal revision. It is not a second ledger. Tool results reuse the neutral AI
call/catalog/display records, and the scoped tool executor constructs presentation
from the checked request. Provider progress cannot inject trusted tool displays.
Payload-bearing diagnostics report codes, identities and byte counts, not plaintext.
Typed failures retain original native causes without formatting them into strings.

`RunJournalWriter` and `JournalReader` are separate narrow ports. A native commit
must atomically validate the expected stream revision and writer fencing epoch,
admit/deduplicate its command, and persist records, checkpoint and exact ciphertext
outbox bytes. A receipt value describes a native durable acknowledgement; constructing
one is not proof of persistence. No provider or target I/O occurs inside a database
transaction. Bounded readers reject gaps, cross-owner records and advertised lengths
before materializing data. Account plus chat is the unique `ChatIdentity`; runner
and key epoch in `StreamBinding` are expected authority, never a second storage
partition permitting the same chat to fork. Cross-runner takeover is not implicit.

Five coordinates have separate meanings: per-run FSM `Revision`, transactional
`StreamRevision`, local `JournalSequence`, public `EventSequence`, and user-facing
`ChatControlRevision`. Token chunks, progress and internal budget reservations do
not advance control revision. Durable command/queue/authority changes do. New prompt
and control commands compare their expected control revision in the same native
transaction as append; two different commands using the same control revision cannot
both be admitted. An identical command returns its original receipt before that
comparison. Reusing its id with changed semantics or attribution conflicts. This
allows streaming progress without continuously invalidating a user's pending prompt.

Fair admission bounds global, account and per-chat queues, rotates accounts, and
serializes mutating target lanes. It starts no worker for an idle transcript.
Expired queued requests are returned for visible durable outcomes. Active leases
remain occupied until explicit cleanup; expiration never pretends a remote process
has stopped. Empty queues are reclaimed, and stale release tokens cannot release a
replacement reservation. Stop/revoke control bypasses the ordinary admission queue.

Canonical signed commands, account authentication boxes, cryptography and hub wire
codecs stay with the hub contract and injected adapters. Local history/storage formats
and existing local AI injection keep their established ownership and representation.

## Alternatives

Putting shared source identities in Cloud would require AI to depend on a crate
that already uses AI records. Copying their definitions into both crates would let
source partitions drift. Placing live scheduling, journals or SDK workers in this
domain would merge pure decisions with native lifecycle and persistence. Treating
provider call ids, progress cursors or arbitrary model metadata as authorization
would allow replay and mutation outside the original grant.

## Consequences

Native adapters and the coordinator receive explicit collaborators and supply
verified observations to these pure owners. Their conformance tests must exercise
real atomicity, fencing, encryption, provider completion and cancellation; structural
DTO checks alone cannot prove them. Shared port fakes belong behind a dev-only
`test-support` feature when actual consumers require them, rather than introducing
another executor or journal source of truth.
