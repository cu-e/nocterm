# 0135. Agent procedures verify responsibility and runtime cost

Status: accepted

## Context

[ADR 0132](0132-proportional-agent-workflows-and-local-resources.md) makes local
work proportional to impact. Structural limits catch growth but cannot establish
cohesion or correct lifecycle. Syntactic field clusters and maximum line counts
can be mistaken for design targets. Duplicated process rules and broad reading
requirements add context while making current decisions harder to locate.

Rendering, repeated background work and caches need explicit correctness and
cost criteria. A cache can reduce computation while retaining excessive memory
or delivering data belonging to an obsolete source.

## Decision

`AGENTS.md` remains the compact entrypoint for mandatory rules and workflow.
The `gate` procedure owns the detailed completion matrix and command reference;
AGENTS requires that matrix. This refines ADR 0132's document organization;
its impact-based verification policy and required checks remain in force.
Roles load specialized procedures for the task rather than preloading all of them.
Reading follows the affected owner and contracts, using supersession links in
the mutable ADR index; accepted records remain unchanged.

Placement first excludes forbidden layers and owners, then seeks a fitting
existing crate. Independent invariants and lifetimes justify extractions.
Survey clusters suggest candidates; they do not determine ownership. Files stay
cohesive as responsibilities emerge, with the existing hard limits as ceilings.
Stateful modules own transitions; pure modules need no artificial wrapper type.
Contract reuse depends on cohesion, dependencies and authority, not whether a
consumer calls every method or a method count crosses a preferred threshold.

The performance procedure applies to changed hot paths. It examines frequency,
work per event and retained memory, removes avoidable work, and requires caches
to have an owner, complete key, invalidation, bounds and teardown. Render/input
callbacks consume prepared state; background work has bounded concurrency and
obsolete results cannot update a different source. Optimization claims identify
representative before/after evidence and distinguish operation counts from
measured latency. Ordinary edits do not acquire an unrelated benchmark gate.

Review covers staged, unstaged, committed and relevant untracked changes. It
examines authoritative state, transitions, cancellation and late delivery as
well as the structural inventory. Characterization tests preserve assertions
and scenarios while allowing mechanical fixture/import relocation.

Claude role guards keep their existing write boundaries. Inline-test edits in
production files are routed to the implementer rather than asking the user to
override a tester role. Policy guards distinguish DTO exemptions from other
architecture-policy changes, including changes to allowed-dependency lists.
Post-edit advice uses recoverable edit context to avoid repeating warnings for
unchanged/shrinking files; unknown context is handled conservatively. These
hooks are tool-specific assistance, with CI and review as the common checks.

## Alternatives

- Lower file limits: adds pressure to split without proving a better owner.
- Cache every repeated operation: ignores invalidation, memory and lookup cost.
- Require profiling and GUI sessions for every edit: spends effort on unaffected
  paths and makes tooling-only changes depend on a product runtime.
- Copy check commands into every role: creates multiple versions of the same gate.

## Consequences

The core guide stays short while specialized procedures carry examples and
commands. Configuration validation includes root instructions, local Markdown
anchors and CLAUDE imports. Behavioral guard tests cover role handoffs and
policy edits. Architectural and performance judgments still require code and
workload evidence; a passing structural check alone establishes neither.
