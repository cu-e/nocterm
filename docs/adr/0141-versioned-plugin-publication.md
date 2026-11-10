# 0141. Plugin records select immutable package versions

Status: accepted

Refines native storage and lifecycle from [ADR0089](0089-user-plugin-contract.md) and
[ADR0090](0090-user-plugin-host-and-passive-providers.md). Guest ABI and consent rules remain.

## Context

Replacing one package file before saving its new hash could leave old consent pointing to
new bytes after a records-write failure or interruption. Native reads and runtime preparation
also blocked GUI operations, while per-contribution polling duplicated window work.

## Decision

The package adapter publishes immutable files named by validated plugin id and content hash.
Records select the exact consented version. Old versions are pruned only after the matching
records receipt succeeds; failed publication leaves restart recovery possible. Native storage
bounds pending versions and installed records separately from enabled runtime instances.

One retained manager serializes bounded admitted operations across windows. Native acquisition,
validation, runtime instantiation and package publication run in the background. The manager
rechecks the consented file hash and installed version before host admission, retains actual
prepared instances, and awaits actual records/data completion. Closing the initiating view does
not discard accepted work. Startup acquires native data before initializing the same pristine
records owner; unreadable startup state stays visible and does not become an empty overwrite.

Plugin data has one authoritative host document and the existing keyed writer. Native reads
use a single consumed preload; the adapter retains completion metadata instead of another
full document. Data removal pauses affected host mutations until its physical completion;
failure leaves the record and old values manageable. An explicitly requested data reset is
its own physical publication and is not rolled back if a later records publication fails.

One driver per physical window schedules the nearest visible contribution deadline. Actual
widget/overflow visibility supplies transitions, panels use checked workspace visibility, and
window teardown releases cadence state. Equal validated trees/styles do not notify again.

## Consequences

Restart restores a hash-bound consented package even after an interrupted replacement.
Preparation caches do not replace actual runtime instances or establish authority. The native
workflow has bounded work and explicit receipts without introducing another persistence loop,
a feature global or an installed helper. Declarative and code package wire formats are unchanged.
