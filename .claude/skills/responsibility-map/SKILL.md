---
name: responsibility-map
description: Read-only analysis of a type, module or crate that finds its responsibilities and candidate state owners from `cargo xtask survey` facts. Use for a split or unclear ownership; early warnings invite inspection rather than mandatory extraction.
user-invocable: false
---

# Responsibility map

Input: a type (`AgentThread`), a module (`agent/thread`) or a crate. Record facts
and the proposed owners in the compact plan (`.claude/skills/feature/plan-bus.md`).
This step writes no source code.

The goal is to find where responsibilities separate **by state**, because an
owner is a set of fields with the transitions that keep them consistent
(AGENTS.md design rules 1, 2 and 7). Line counts signal remaining room; they do not decide whether or where to split.

## 1. Collect facts

Use a survey when it helps resolve ownership (Cargo resources follow CONTRIBUTING.md):

```sh
cargo xtask survey <package> --json > .claude/plans/<slug>/survey.json
cargo xtask survey <package> --type <Type>
```

Record relevant facts in the plan: the warnings, the largest files, the type's fields,
methods and impl files, and the module reference counts. The survey is
syntactic: it keys types by name and does not expand macros, so confirm any
surprising fact in the source.

## 2. Read the matrix

The `--type` matrix lists, per method, the fields reached through `self`, and
groups fields that the same methods use (read or write) together. Fields only
coordinators use are listed separately: assign them by hand.

| Pattern | Meaning | Next step |
| --- | --- | --- |
| A cluster of 2+ fields with its own methods | possible owner | confirm one invariant and lifetime before `owner-extract` |
| Every field is its own cluster and most methods are cross-cluster or coordinators | possible coordination among existing owners | inspect actual types and transitions; split by use case only if responsibilities differ (ADR 0044) |
| Many methods with no direct field access | delegating wrappers or free functions | move each next to the owner it calls, or into a pure function of the domain |
| A field no method touches | state used only through destructuring or by another type | confirm; it may belong to that other type |
| Impl blocks of one type in 4+ files | possible file slicing | inspect responsibilities; do not treat each existing file as a future owner |

Clusters reflect syntactic co-access, not semantic ownership. Confirm which
fields must change together, who is authoritative, and when work ends. A cohesive
type may need no split; pure logic may need functions rather than a state owner.

For a module or crate, use the `modules` section: a top-level module whose
references go mostly to one workspace crate, and which has few references to
sibling modules, is a candidate crate. Check its layer with `placement` before
proposing it.

## 3. Name each candidate

For every candidate owner write:

- a noun for **what it owns** (`PromptQueue`, `SessionGrants`), not what it does
  (`QueueHelper`, `ThreadPart2`);
- a draft card: `Owns:`, `Does not own:`, `Invariants:`;
- its fields, its transition methods, and what it returns to the parent (a value,
  a typed error, or a named effect for the coordinator to apply);
- creation/teardown, cancellation and the destination of late asynchronous results;
- expected field counts after extraction for both the parent and the owner.

Reject a candidate that would need the parent passed back into it (`&mut Parent`,
the parent's `Context`): it is not a separate responsibility yet.

## 4. Card drift

Compare code with the crate card. A conflicting responsibility needs a placement
fix; do not legitimize it by expanding the card. Use `docs-adr` for actual boundary
decisions and stale descriptions after ownership is resolved.

## Decision to record

```
# Responsibility map: <subject>
## Facts (survey <date>)
## Interpretation (matrix pattern, evidence)
## Candidates
### <Owner name>
- Fields: …   Methods: …   Returns: …
- Card: Owns … / Does not own … / Invariants …
- Parent fields after: N → M
## Card drift
## Recommended order (smallest safe extraction first)
```

Report the map and next useful step; `behavior-tests` and `owner-extract`
apply when an extraction is needed.
