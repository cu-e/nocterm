# 0066. A scoped chat declares bounded authority before its payload

Status: accepted. Extends [ADR 0022](0022-agent-storage-owners.md) and
[ADR 0037](0037-chat-document-semantic-revisions.md).

## Context

A flat transcript directory is discovered by older binaries and cannot separate
project history from General. Rebuildable sidecars cannot authorize a scope or
safely supply another project's titles. Reading every transcript body to validate
its catalog metadata defeats lazy history discovery. Arbitrary valid legacy v1
JSON may place that metadata after the entries, so its compatibility cost differs.

## Decision

AI owns SavedHeader, the checked ContextId membership and version-aware pure codec.
Canonical v1 encoding remains byte-compatible and belongs to General. V2 requires
an explicit valid scope and uses the envelope version/catalog/chat. Unknown,
missing, malformed or contradictory ownership never falls back to General.

The v2 catalog projection precedes the payload and is bounded to 16 KiB including
escaped JSON framing. It contains only ID, agent ID, scope, title, name, pin,
updated, model and content presence. Display fields and fallback titles have
bounded projections reflecting the actual persisted payload, including image
budget replacements. The same serializer defines stored bytes and byte budgets.

The transcript header is authoritative. Discovery can expose its recoverable
metadata without claiming to validate the unread body. Full hydration compares
the body's projected header and rejects contradictions before delivery. Loading
validates the captured requested scope and UUID from the same held source's prefix
before reading its remaining payload; it does not reopen a checked pathname.
Sidecars are versioned rebuildable caches, verified against that authority before
use. A cache failure cannot undo a published transcript.

`nocterm-agent-storage` places v2 documents in scoped-v2/general or
scoped-v2/projects/ProjectId. Catalog/load/delete use a captured expected scope;
save derives it from the saved header. Identical chat UUIDs in distinct scopes are
independent physical documents. Project discovery never scans General, another
project or the flat legacy directory. The 200-row limit applies within the
requested scope after authority checks.

General preserves valid arbitrary v1 documents and their exact wire representation.
Their metadata may require a bounded full scan, including on a warm cache; no
sidecar is trusted to avoid that scan. A scoped General document takes precedence
over a legacy duplicate, even if damaged. Delete validates both records and
removes legacy first so a failed removal cannot resurrect stale fallback content.

The app injects session LocalFs. Generated descendants, reads, staging and removal
use held LocalDirectory capabilities. Only configured application-owned private
root ancestry is trusted/canonicalized. Project storage requires supplied held
identity and refuses a portable path-only implementation. General retains the
portable adapter's documented active-ancestor limitation. LocalStage supplies
private atomic publication and acknowledged cancellation; there is no second
native staging engine or persistence loop.

## Alternatives

A scope sidecar outside the transcript could mislabel a copied or stale document.
Putting v2 in the old flat directory lets older discovery expose project metadata.
Trusting file size/time stamps cannot validate ownership. Rewriting all legacy
records or rejecting legal v1 field ordering solely to obtain bounded-prefix
performance breaks compatibility. Scanning all scopes to distinguish foreign
UUIDs leaks membership and prevents independent scope keys.

## Consequences

Project records survive as scoped data when a definition is unavailable; they do
not become General. Runtime/document/roster ownership must capture the same scope
before asynchronous work. Catalog discovery retains bounded v2 metadata work,
while valid legacy v1 and explicit selected-document hydration retain their stated
bounded full-read costs. Shared history permits and resident budgets stay owned
by the installation rather than multiplied by visible context count.
