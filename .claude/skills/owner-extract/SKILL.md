---
name: owner-extract
description: Extract a confirmed state responsibility into a private owner with transition methods and behavior coverage. Use when ownership requires a split; approaching a size limit prompts inspection, not mechanical extraction.
user-invocable: false
---

# Owner extraction

Input: a confirmed candidate in the compact plan. Record the change and evidence
there (`.claude/skills/feature/plan-bus.md`); separate stage files are optional.

This is the transformation AGENTS.md design rules 1, 2 and 7 ask for. Moving
methods of the parent into another file is not an extraction and fails review.

## Preconditions

- The plan names the owner, state, invariant, transitions, lifetime and results.
  Joint field access or a lower field count alone does not justify the owner.
- Characterization tests exist for the behaviour that goes through those
  methods (`behavior-tests` in characterize mode). If they do not, write them
  first and see them pass on the unchanged code.

## Procedure

1. **Create the owner** in a module named after it, beside the parent:
   - private fields only; construction through `new`/a builder that establishes
     its invariants;
   - methods are transitions: they take plain inputs, change only the owner's
     state, and return a value, a typed error (`typed-errors`) or a named effect;
   - no reference back to the parent, its `Context`, or a GPUI entity it does
     not own. If GPUI work is needed, the parent performs it with the returned
     effect.
2. **Hold it in the parent** as one field replacing the fields it absorbs.
3. **Move the logic**, one method at a time. A parent method either disappears
   (callers use the owner through the parent's narrow accessor) or becomes a
   short coordinator call that applies the returned effect.
4. **Delete the moved fields** from the parent. Do not leave mirrors or
   `pub(crate)` getters that expose the owner's internals.
5. **Test the owner directly** through its methods for invariants; preserve the
   characterization scenarios and assertions. Mechanical imports/fixtures may move.
6. Use `gate` scoped mode for the owner/characterization behavior after the
   coherent extraction when needed. Reuse unchanged results; no command bundle
   after each moved method and no full gate for each extracted owner.

## Done when

- The parent's field count dropped by (absorbed fields − 1).
- The parent's impl-file count did not grow (`cargo xtask survey <package> --type <Parent>`).
- The owner has no `pub`/`pub(crate)` fields and does not take the parent.
- The owner's module name is a noun for what it owns.
- Characterization behavior/expectations remain intact and pass; owner invariants
  have coverage. Explain any fixture/API adaptation without weakening assertions.
- `xtask/baseline.toml` entries went down or stayed equal; lower any that went
  down (the check fails until you do).

## Not an extraction

| Change | Why it fails |
| --- | --- |
| New file with `impl Parent { … }` only | same type, more impl files, no new invariants |
| Owner with one field and pass-through methods | a facade; the state still belongs to the parent |
| Owner methods taking `&mut Parent` or `Context<Parent>` | the responsibility did not separate |
| Fields moved but the parent keeps setters for each | the parent still owns the transitions |
| Splitting a function into `step1`/`step2` helpers | complexity moved, not reduced |

Report behavior evidence and before/after field counts in the shared plan or handoff.
