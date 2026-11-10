---
name: placement
description: Decide which crate and layer new or moved code belongs to, and whether it needs a new crate, before any code is written. Used by the crate-new, crate-extend, refactor and crate-split workflows and by the architect agent.
user-invocable: false
---

# Placement

Input: one responsibility in a sentence ("parse container listings", "retain
chat drafts across restarts"). Record the decision in the compact plan
(contract: `.claude/skills/feature/plan-bus.md`).

The rules are AGENTS.md "Where code goes" and docs/ARCHITECTURE.md layers; this
is the procedure for applying them.

## 1. Classify the responsibility

Classify each responsibility using the layer table; separate mixed responsibilities.

| Question | Layer |
| --- | --- |
| Is it a reusable low-level primitive with no product policy? | foundation, within its allowed dependencies |
| Is it a rule, format or contract with no I/O and no GPUI? | domain |
| Does it perform real filesystem, network, process, D-Bus or codec I/O behind a contract? | adapter |
| Is it a GPUI entity with background work and no view? | service |
| Is it a view, presentation state or user interaction? | feature (or `ui` for shared components) |
| Does it compose the above? | app (`src/bootstrap.rs`) |

Split a responsibility that answers yes to two questions (for example "read the
file and validate it") into one piece per layer before going on.

## 2. Find the candidate crate

1. Find candidate owners in `docs/architecture/dependencies.md`; read their
   relevant layer-note sections, not every subsystem in the layer.
2. For each candidate crate, read the `//!` card in its `lib.rs` (or `main.rs`).
   The responsibility must fit `Owns:` and must not appear in `Does not own:`.
3. Check the dependencies the code needs against the layer table in
   docs/ARCHITECTURE.md. A feature never depends on another feature: features
   meet through a narrow workspace port (`port-design` skill).
4. Search the ADR index (`docs/adr/README.md`) for decisions governing the
   changed contracts. Follow supersession links; read older context when needed
   to resolve a constraint. An incidental crate mention is not a reading requirement.

## 3. Module or new crate

First exclude candidates whose layer/dependencies or ownership card forbid the
responsibility. One such conflict is enough; never put it there as a module.
Reuse a fitting existing crate. Propose a new crate when no existing owner fits;
justify its cohesive responsibility, dependency boundary and consumers.

Within a fitting crate, use a responsibility-named module (never `utils`,
`helpers`, `common`, `misc`). Independent invariants, lifetimes and reasons to
change guide separation; line counts or independent tests alone do not decide it.
Compare meaningful alternatives by correctness, reuse, coupling and runtime cost.

Existing mixed responsibilities are migration debt, not precedent: "agent
already does X" is not a reason to put more X there.

## 4. Boundary decisions

- If the code needs a dependency its layer forbids, resolve the boundary within
  the authorized design (move a narrow contract down, split the responsibility,
  or choose a fitting crate). Ask only when a material choice or scope change
  remains; never introduce a forbidden dependency.
- The decision changes a crate boundary or a port between layers → mark
  "ADR needed" for the `docs-adr` skill.

## Decision to record

```
# Placement: <responsibility>
- Layer: <layer> — <answer that decided it>
- Crate: <crate> (new | existing) — <reason>
- Card check: fits Owns "<quote>"; not in Does not own
- Dependencies needed: <list>, allowed by layer: yes/no
- ADRs read: <numbers>; ADR needed: yes/no — <why>
- Module: <module name> — owns <state/behaviour>
```

Report placement and any material unresolved decision in the shared plan.
