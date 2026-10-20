---
name: responsibility-map
description: Read-only analysis of a type, module or crate that finds its responsibilities and candidate state owners from `cargo xtask survey` facts. Run before any split, owner extraction or crate split, and when a file or type reaches an early-warning threshold.
user-invocable: false
---

# Responsibility map

Input: a type (`AgentThread`), a module (`agent/thread`) or a crate. Output:
`20-map.md` in the plan directory (`.claude/skills/feature/plan-bus.md`). This
step writes no source code.

The goal is to find where responsibilities separate **by state**, because an
owner is a set of fields with the transitions that keep them consistent
(AGENTS.md design rules 1, 2 and 7). Line counts only say that a split is due;
they never say where.

## 1. Collect facts

Run through the shared lock (AGENTS.md hard rules):

```sh
.claude/hooks/cargo-serial.sh cargo xtask survey <package> --json > .claude/plans/<slug>/survey.json
.claude/hooks/cargo-serial.sh cargo xtask survey <package> --type <Type>
```

Record in `20-map.md`: the warnings, the largest files, the type's fields,
methods and impl files, and the module reference counts. The survey is
syntactic: it keys types by name and does not expand macros, so confirm any
surprising fact in the source.

## 2. Read the matrix

The `--type` matrix lists, per method, the fields reached through `self`, and
groups fields that the same methods use (read or write) together. Fields only
coordinators use are listed separately: assign them by hand.

| Pattern | Meaning | Next step |
| --- | --- | --- |
| A cluster of 2+ fields with its own methods | a hidden owner | `owner-extract` |
| Every field is its own cluster and most methods are cross-cluster or coordinators | the fields are already owners; the type is a coordinator with too large a surface | split coordination by use case: named effects returned by owners (ADR 0044) or a separate coordinator per workflow |
| Many methods with no direct field access | delegating wrappers or free functions | move each next to the owner it calls, or into a pure function of the domain |
| A field no method touches | state used only through destructuring or by another type | confirm; it may belong to that other type |
| Impl blocks of one type in 4+ files | file slicing already happened | treat each file's methods as a candidate owner, check its fields |

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
- expected field counts after extraction for both the parent and the owner.

Reject a candidate that would need the parent passed back into it (`&mut Parent`,
the parent's `Context`): it is not a separate responsibility yet.

## 4. Card drift

Compare what the code does with the crate card. List every responsibility that
is under `Does not own:` or missing from `Owns:`; these go to `docs-adr`.

## Output: `20-map.md`

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

End with the Result block; `next` is usually `behavior-tests` (characterize)
and then `owner-extract`.
