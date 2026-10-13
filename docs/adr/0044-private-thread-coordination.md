# 0044. Private chat owners return named coordination effects

## Context

AgentThread combined disposable agent-session state, terminal work, durable
documents and presentation bindings. Its operations mutated shared state from
callbacks spread across many implementation files. Relocating those methods
alone would preserve the coupling and make original authority difficult to audit.

## Decision

AgentThread composes nine private collaborators: SessionDriver, ToolRuntime,
Archive, Composer, ChatConfig, ChatDocument, ThreadNotice, ChatHost and
ThreadBindings. Four coordinator files route lifecycle, incoming events and
completions, durable input/history actions and outgoing dispatch. Other modules
own their types, preparation, bounded operations or pure framing.

SessionDriver owns the lease, phase, visibility, restore information, reported
agent information and original outstanding-operation counter. Opening,
authentication, prompt and cancellation operations retain their original
authority. Late opening completion cleans up through its original commands;
releasing a lease cannot replace the counter held by accepted work.

ToolRuntime composes approval admission, fixed turn context and opaque identities,
structured command jobs, live observations, terminal handoff receipts and display
correlation. Finite dispatch, reconnect, connection-wait, command-wait and live
operations retain their accepted capabilities and timing state. State owners
expose named transitions and immutable projections, not mutable bags or getters.
Workspace observations are explicit finite inputs to attachment projection.

Each weak chat update invokes a named probe, action or completion. Native
background creation validates the original chat, registration, attachment and
workspace immediately before creation in the same application transaction.
Its placement receipt remains available for cleanup after chat disappearance;
cleanup preserves promoted tabs and replacement epochs. Secret waits retain the
original prompt token. Live completion retires only its matching lease, and
structured jobs compare their original executor with supplied current facts.

Configuration and archive operations return their captured request ticket or
local load generation and delivery permit. The coordinator validates them before
applying owner transitions. Queue preparation remains nonmutating until complete
document admission succeeds. Runtime client events remain synchronous and FIFO;
physical publication, admission and durable semantic revisions remain distinct.

ThreadNotice owns status text and error emphasis, including text-only changes
that retain an existing failure. ThreadBindings retains the originating window
and subscription lifetimes without performing session or persistence effects.
Release keeps its established ordering: finalize pending work, capture durable
intent, cancel execution, release the lease, then perform deferred registration
and original background cleanup.

These are private feature values tied to the existing chat lifetime, not new
entities or independent services. Native storage, agent lifecycle admission and
the shared persistence/shutdown driver retain their existing owners. No worker,
global lookup site, quit handler or deadline is added.

## Alternatives rejected

- Concatenating coordinator methods into fewer files retains shared leaf state.
- A callback receiving the whole mutable chat bypasses named owner invariants.
- Resolving current authority for an old approval silently renews its permission.
- Separate composer or operation entities introduce unnecessary ordering seams.
- A general environment registry hides collaborators and permits service lookup.

## Consequences

The coordinator has bounded private state and four implementation files. Finite
operations preserve original authority and cleanup ownership while leaf owners
remain responsible for actual transitions. Notice/window changes preserve
existing observable behavior. Version-one ACP history, compatibility globals and
panel ownership remain separate boundaries; this decision changes none of their
formats or policies.
