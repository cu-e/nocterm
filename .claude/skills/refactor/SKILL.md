---
name: refactor
description: Restructure within a crate without changing behavior, using responsibility facts and characterization tests; use for a needed extraction or requested cleanup.
argument-hint: "<type | module | file> [package] [target: master|dev]"
---

# Refactor within a crate

Subject: $ARGUMENTS

Follow AGENTS.md's Workflow and Definition of done, using a compact plan or
[structured handoff](../feature/plan-bus.md) as useful. Record branch/base,
behavior to preserve and a relevant structural metric. An early warning alone
does not require this workflow.

1. Use `responsibility-map` to identify real owners. If fields already represent
   owners, consider coordination by use case or named effects (ADR 0044).
   Ownership moving to another crate uses `crate-split` instead.
2. Design extractions in a safe order, with expected counts, characterization
   coverage and a `docs-adr` decision. Apply AGENTS.md's authorization policy;
   routine details within an authorized design do not require another approval.
3. Characterize behavior on unchanged code, then `owner-extract` one responsibility
   at a time. Preserve behavior; report any behavior change needed outside scope.
4. Use `gate` for affected behavior after coherent edits, reusing valid evidence.
   Lower/remove debt inventory entries when the measured debt shrinks.
5. Review with `design-review`, fix findings and finish the applicable input-based
   completion checks. Report before/after metric, behavior evidence and limitations.
   Commit/push/PR only when requested.
