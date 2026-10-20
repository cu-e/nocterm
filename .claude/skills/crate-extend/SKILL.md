---
name: crate-extend
description: Workflow for adding behaviour to an existing crate — confirm it fits the crate's card, check the touched files and types against early warnings, make room with a refactor first if needed, then build, verify and review. Use for most features and non-trivial changes inside one crate.
argument-hint: "<change> [crate] [target: master|dev]"
---

# Workflow: extend a crate

Task: $ARGUMENTS

You orchestrate; pass the plan directory, target branch and verification scope
to every subagent (`.claude/skills/feature/plan-bus.md`).

## 0. Task

Write `00-task.md`: change, target, branch `<type>/<slug>`, verification scope,
success metric (the behaviour, plus "no new early warnings for touched files"
or the specific counts if a split is needed).

## 1. Design (architect)

1. `placement` → `10-placement.md`. If the change belongs to another crate or a
   new crate, switch to that workflow; if it spans layers, split it per layer.
2. Survey the target: `.claude/hooks/cargo-serial.sh cargo xtask survey <package>`.
   For each file or type the change will grow, check its early warning:
   - a type that would exceed 12 fields, a file that would exceed 640 lines or a
     type in 4 impl files → run `responsibility-map` and plan the extraction
     **before** the feature (it becomes the first steps, or a separate
     `refactor` run on its own branch if large);
   - under the thresholds → continue.
3. `port-design` for any new collaborator; `typed-errors` for new fallible APIs;
   `wiring` for new settings, persistence or installation changes.
4. `docs-adr` decision (usually: card update only).
5. `30-design.md` including expected field and line counts after the change.

Show `30-design.md` to the user and wait for approval.

## 2. Build (implementer)

Make room first (`behavior-tests` characterize → `owner-extract`), then the
change with `behavior-tests` cover. `gate` quick after each step; `40-steps.md`.

## 3. Verify (tester) — `gate` full; add missing tests.

## 4. Review (reviewer) — `design-review` → `60-review.md`; two-round limit.

## 5. Wrap up

Report gate results, verdict, baseline delta, early warnings before/after, and
proposed commits (refactor commits separate from the feature commit). Commit
only if the user asked.
