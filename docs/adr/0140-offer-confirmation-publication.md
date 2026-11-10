# 0140. Displayed key confirmations await native publication

Status: accepted

Refines the completion contract of [ADR0114](0114-native-captured-account-operations.md)
and uses [ADR0015](0015-private-file-publication.md) for file commit semantics.

## Context

Displayed administrator and recovery offers retain their original recipient, but synchronous
confirmation performed cache encoding and filesystem synchronization while holding authority
locks. GUI status checks shared those locks. Administrator pin writes could fail after the
confirmation had already appeared successful.

## Decision

Administrator and recovery confirmation ports return runtime-neutral futures. The GUI retains
one recovery acknowledgement operation, waits for completion before advancing sign-in, and
reports typed persistence failures. Original recipient capabilities pass through unchanged.
A retry cannot select a replacement account by its descriptive hub id.

Native account cache preparation uses a shared finite blocking-work budget. It stages bounded
immutable snapshots and syncs their bytes outside authority locks. Publication rechecks the
original membership/open-key identity and exact content generation. Only the namespace commit
and matching state transition occur under admission; parent-directory sync follows outside.
`AtomicPublication::commit` returns a receipt whose drop cannot roll back the committed file.
The existing `publish` convenience method preserves commit plus best-effort parent sync.

Security facts (key installation, administrator pins and recovery acknowledgement) complete
only after successful publication. Ordinary encrypted refresh may accept an authority-checked,
bounded memory snapshot when an offline-copy I/O write fails; an oversized candidate is refused
and the last readable disk file survives. Failed remote-first key creation keeps an unconfirmed
marker only within its original live membership so a repaired write can offer reissue.

## Consequences

Successful confirmation means the native publication completed, rather than a queued save.
It retains ADR0015's best-effort directory sync and does not promise power-loss durability.
A failed disk write cannot create a durable recovery marker; replacement membership or restart
invalidates its ephemeral marker. Initial configured cache acquisition occurs before GUI
startup; hot key-status queries inspect loaded Slots only.
