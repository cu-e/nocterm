# 0037. Durable chat transitions and capture have separate owners

## Context

AgentThread mixed durable identity, transcript, revision, budget and pending-save
state with session coordination. Snapshotting and capture advanced revisions even
for untouched imported chats. Publication failures lacked their actual revision,
and accepted registry stamps could precede writer admission.

## Decision

A private non-entity ChatDocument in the agent feature owns durable identity,
metadata, transcript, semantic revisions, pending immutable candidates, typed
revision-bearing failures, bounded budgets and draft debounce. Composer and
ChatConfig remain separate owners. AgentThread interprets their named transitions.
Native publication remains in agent-storage; runtime and Store own ordered delivery.

Borrowed comparisons identify actual changes within the version-one saved row
window. Metadata projections share literal draft and queued Arc payloads. They
retain no second transcript/media or JSON baseline. Accepted A→B→A transcript
transitions remain semantic activity; a generation is not an equality fingerprint.

Clean import, browsing, equal resumed connection, read-only projection and capture
do not advance revisions or timestamps. True accepted mutations advance their
revision before timestamping. Compound queue admission validates the complete
candidate before either owner changes. Dispatch waits for physical publication of
the desired queued document; an offer or admission alone cannot release a prompt.

Deferred offers coalesce without claiming admission. Capture offers existing intent
without consuming it. Runtime commits its accepted owner/revision stamp only after
KeyedPersisted owns the immutable snapshot, rejects older revisions from the same
owner and permits same-revision retry. SaveAdmitted, Saved and SaveFailed distinguish
admission from physical outcome. Old acknowledgements cannot release newer input,
cancel its draft timer or clear its failure. Retry retains the original snapshot
and revision. Shutdown joins admission and publication failures while preserving
writer-owned intent when the client has nothing pending.

Normal removal captures before dropping the strong document owner. Failed admission
retains its owner and registration. Detach admission acknowledges the original
client after the Runtime borrow ends, even though it has been unregistered; the
feature validates the exact candidate revision. That acknowledgement cannot publish
another payload or grant authority to a replacement. Permanent deletion and stale
ownership remain authoritative.

AI disable finalizes session changes before capturing durable intent. Disabled live
transcript and composer data may be released only when settled or when the current
immutable candidate is admitted; unadmitted changes and failures retain their input.
The admitted candidate retains everything needed for physical retry. Ordinary
archive eviction still requires settled durable state. Hydration preserves an
already accepted draft overlay's revision and timestamp; a subsequent changed
provider resume is a separate semantic mutation.

Supplied-identity snapshot and budget framing avoids throwaway UUID/time calls.
Genuine identity allocation and legacy transcript timestamps retain their existing
native domain debt. The persisted history remains version one and ACP-backed.

## Alternatives rejected

- Treating every save or capture as a mutation rewrites untouched history.
- Retaining a second transcript/media baseline duplicates bounded resident data.
- Hashing or re-encoding whole transcripts does not provide exact borrowed equality.
- Marking an offered candidate admitted loses shutdown-before-deferred-save intent.
- A synchronous callback during Runtime update permits reentrant entity borrowing.
- Clearing disabled input before writer ownership can lose a rejected candidate.

## Consequences

Feature owners describe semantic state; runtime describes delivery identity and
Store owns physical execution. Clean version-one files retain their original bytes.
This decision adds no new writer, global, quit hook, neutral history conversion or
version migration. Session and tool coordination remain separate ownership work.
