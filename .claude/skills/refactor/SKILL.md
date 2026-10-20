---
name: refactor
description: Workflow for a behaviour-preserving restructuring inside a crate — map responsibilities from survey facts, pin behaviour with characterization tests, extract owners one at a time, lower the debt baseline. Use when a file or type hits an early warning or limit, or the user asks to split, clean up or restructure code within a crate.
argument-hint: "<type | module | file> [package] [target: master|dev]"
---

# Workflow: refactor within a crate

Subject: $ARGUMENTS

Behaviour does not change. Pass the plan directory, target branch and
verification scope to every subagent (`.claude/skills/feature/plan-bus.md`).

## 0. Task

`00-task.md` with a measurable metric taken from the survey, for example
"`<Type>` fields 14 → ≤ 9; files ≥ 640 lines in `<module>/`: 2 → 0;
`impl_spread` entry for `<Type>` 6 → ≤ 4". Branch `refactor/<slug>`.

## 1. Map (architect)

1. `responsibility-map` → `20-map.md`. Its matrix pattern decides the route:
   - hidden owners → step 2 per candidate;
   - fields are already owners, coordination is the problem → design named
     effects or per-workflow coordinators (ADR 0044) in `30-design.md`;
   - a module that would rather be a crate → switch to `crate-split`.
2. `30-design.md`: candidates in order (smallest safe extraction first), each
   with its expected counts; characterization tests to write; ADR decision via
   `docs-adr` (none if crate boundaries hold).

Show the map and design to the user and wait for approval.

## 2. Extract (implementer), per candidate

1. `behavior-tests` characterize — pass on unchanged code; names recorded.
2. `owner-extract` — one owner per step.
3. `gate` quick; tick the step in `40-steps.md` with before/after counts.

Never mix behaviour changes into these steps. If a step needs one, stop and
report.

## 3. Verify (tester)

`gate` full. Rerun the survey and compare with the metric in `00-task.md`;
characterization tests unchanged and green.

## 4. Review (reviewer) — `design-review`; two-round limit.

## 5. Wrap up

Report the metric before/after, gate results, baseline entries lowered and the
verdict. One `refactor(<scope>): …` commit per extracted owner is preferred.
Commit only if the user asked.
