---
name: owner-extract
description: Extract one responsibility of a type into an owner type with private state and transition methods, behind characterization tests. Use for a candidate from responsibility-map, when a type nears 15 fields, or when a file nears 800 lines — never cut a file or move methods of the same type instead.
user-invocable: false
---

# Owner extraction

Input: one candidate from `20-map.md`. Output: source changes and an updated
`40-steps.md` (`.claude/skills/feature/plan-bus.md`).

This is the transformation AGENTS.md design rules 1, 2 and 7 ask for. Moving
methods of the parent into another file is not an extraction and fails review.

## Preconditions

- `20-map.md` names the owner, its fields, its methods and what it returns.
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
5. **Test the owner directly** with unit tests for its invariants; keep the
   characterization tests unchanged and green.
6. Run `gate` in quick mode.

## Done when

- The parent's field count dropped by (absorbed fields − 1).
- The parent's impl-file count did not grow (`cargo xtask survey <package> --type <Parent>`).
- The owner has no `pub`/`pub(crate)` fields and does not take the parent.
- The owner's module name is a noun for what it owns.
- Characterization tests pass without edits; the owner has its own tests.
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

End with the Result block and the before/after field counts.
