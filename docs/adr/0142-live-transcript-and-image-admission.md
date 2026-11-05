# 0142. Admit live transcript and image growth before allocation

Status: accepted

Refines image preparation in [ADR0035](0035-injected-image-codecs.md) and the
document transitions in [ADR0037](0037-chat-document-semantic-revisions.md).

## Context

Per-image and per-protocol-line limits do not bound an aggregate selection or a
long-lived conversation. Checking only the completed collection allows rejected
work to retain decoded images and encoded sources. Preview keys based on an
unretained allocation address can also alias a different image after reuse.

## Decision

The AI domain owns immutable shared encoded image data, serialized as the same
string as before. Construction normalizes excess capacity once. Projections and
previews share that allocation; a preview retains its complete source identity.

Image collection rules admit count and remaining bytes before native acquisition,
decode and base64 encoding. One injected image-work owner bounds physical work
across attachment preparation, queue reconstruction and previews to four jobs.
Actual work retains its permit through cancellation of the initiating view.
Preview residency is separately bounded to 32 entries and 64 MiB, with source,
PNG and RGBA charges, finite in-flight reservations and generation-bound delivery.

The live local transcript admits growth before mutation with incremental byte
and row accounting, including text, tools and control metadata. Its ceilings are
64 MiB and 20,000 rows. Refusal preserves accepted rows and does not consume an
unaccepted queued prompt. Saved-history limits remain a separate contract.

Cloud reduction prepares a transactional candidate on the background executor.
Raw retained costs update for changed or appended rows, including large images
that saved-history filtering would omit. Delivery checks the original projection
identity and captured source. Textual candidate copying remains part of that
immutable contract; encoded image bytes are shared.

## Consequences

Aggregate admission bounds rejected work before its expensive allocations.
Physical work bounds survive dropped waiters, while preview eviction releases
retained sources. These bounds are conservative accounting policies rather than
measurements of total process RSS. Wire formats and captured-source authority
are unchanged.
