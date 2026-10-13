# 0047. Connection publication service preserves committed profile authority histories

## Context

The connection feature owned checked profile publication and coupled recents
capture alongside editor inputs and views. Detected-fact consumers need authority
that distinguishes intermediate changes even when a coalesced document returns to
identical serialized data. Reading persistence from its synchronous publication
hook would reenter a leased GPUI entity.

## Decision

`nocterm-connections-store`, a GPUI-only service, owns checked profile admission,
profile/recent persistence composition and coupled capture. The feature aliases
the actual `ConnectionStore` entity and retains editor inputs, session conversions,
views and detected-fact lifecycle. The app supplies native writers, a clock and the
existing shutdown coordinator. No feature or adapter dependency enters the service.

Pure `ObservationEpochs` in the connection domain applies every successful
candidate transition using supplied monotonic generations. Its live map carries
incarnation, exact endpoint and independent manual icon/country stamps. Deletion
prunes the row; re-addition allocates fresh stamps. Endpoint and override cycles
retain their intermediate history. Presentation, credential and order changes
preserve unrelated stamps. Checked revision exhaustion rejects the candidate
without wrapping. Imported IDs initialize deterministically; stamps are neither
serialized nor persistent tombstones.

Immutable `ProfilePublication` contains a generation and live authority rows,
without another complete profile document. The supplied committed snapshot hook
first applies derived recents and then replaces the distinct `ProfilePublications`
carrier before completing the successful receipt. It does not reread profile
persistence, capture a feature callback or schedule native work.

Durable profile and optimistic recents publication reuse the common persistence
driver. Profile snapshots include generation, effects and stamps in equality;
writers serialize only profiles. Rejected or failed edits publish no authority.
Capture freezes immutable snapshots and uses the existing two-notification feed
for physical prefix/final recents effects under the common 180-ms deadline.
Background drain has no GPUI access; teardown does not authorize new observation
jobs. Independent recents survive profile failure, and a later final failure
cannot revoke a successful prefix.

## Alternatives rejected

- Moving product publication policy into generic storage widens its ownership.
- A feature facade retaining mutable publication state preserves mixed ownership.
- Final serialized equality loses delete/re-add and manual-override history.
- A new writer or capture deadline duplicates the existing shared lifecycle.
- Calling feature/native observation code from a leased hook creates reentrancy
  and service-to-feature coupling.

## Consequences

Profile/recent wire formats, comment semantics, checked edits and receipt ordering
remain compatible. Installed handles and credential callbacks retain the same
actual service entity. Detected-fact scheduling, admission and stale-completion
guards remain a separate responsibility; this service provides committed authority
without implementing observation jobs.

This supersedes feature ownership of profile/recent publication described in
ADR0013,0036 and0043. Their shared driver, capability and retained-installation
contracts continue to apply; accepted historical records remain unchanged.
