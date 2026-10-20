---
name: placement
description: Decide which crate and layer new or moved code belongs to, and whether it needs a new crate, before any code is written. Used by the crate-new, crate-extend, refactor and crate-split workflows and by the architect agent.
user-invocable: false
---

# Placement

Input: one responsibility in a sentence ("parse container listings", "retain
chat drafts across restarts"). Output: `10-placement.md` in the plan directory
(contract: `.claude/skills/feature/plan-bus.md`).

The rules are AGENTS.md "Where code goes" and docs/ARCHITECTURE.md layers; this
is the procedure for applying them.

## 1. Classify the responsibility

Answer in order; the first "yes" decides the layer.

| Question | Layer |
| --- | --- |
| Is it a rule, format or contract with no I/O and no GPUI? | domain |
| Does it perform real filesystem, network, process, D-Bus or codec I/O behind a contract? | adapter |
| Is it a GPUI entity with background work and no view? | service |
| Is it a view, presentation state or user interaction? | feature (or `ui` for shared components) |
| Does it compose the above? | app (`src/bootstrap.rs`) |

Split a responsibility that answers yes to two questions (for example "read the
file and validate it") into one piece per layer before going on.

## 2. Find the candidate crate

1. Read `docs/architecture/dependencies.md` and the layer note for the layer.
2. For each candidate crate, read the `//!` card in its `lib.rs` (or `main.rs`).
   The responsibility must fit `Owns:` and must not appear in `Does not own:`.
3. Check the dependencies the code needs against the layer table in
   docs/ARCHITECTURE.md. A feature never depends on another feature: features
   meet through a narrow workspace port (`port-design` skill).
4. Search the ADR index (`docs/adr/README.md`) for the subject and read every
   accepted record that names the candidate crate.

## 3. Module or new crate

Propose a new crate when **two or more** hold:

- it needs a dependency set the candidate crate must not or does not have;
- it belongs to a different layer than the candidate crate;
- it changes for different reasons and has independent tests;
- adding it would put it in the candidate's `Does not own:` or grow the card's
  `Owns:` with an unrelated subject.

Otherwise it is a module of the existing crate, named after the responsibility
its type owns (never `utils`, `helpers`, `common`, `misc`).

Existing mixed responsibilities are migration debt, not precedent: "agent
already does X" is not a reason to put more X there.

## 4. Stop conditions

- The code needs a dependency its layer forbids → `escalate` with the options
  (move a narrow contract down a layer, split the responsibility, new crate).
- The decision changes a crate boundary or a port between layers → mark
  "ADR needed" for the `docs-adr` skill.

## Output: `10-placement.md`

```
# Placement: <responsibility>
- Layer: <layer> — <answer that decided it>
- Crate: <crate> (new | existing) — <reason>
- Card check: fits Owns "<quote>"; not in Does not own
- Dependencies needed: <list>, allowed by layer: yes/no
- ADRs read: <numbers>; ADR needed: yes/no — <why>
- Module: <module name> — owns <state/behaviour>
```

End with the Result block from the plan contract.
