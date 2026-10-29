---
name: crate-new
description: Workflow for adding a new workspace crate — placement proves a new crate is needed, the design fixes its card, ports, errors and wiring, then build, verify and review. Use when a responsibility has no fitting crate or the user asks for a new crate.
argument-hint: "<responsibility in one sentence> [target: master|dev]"
---

# Workflow: new crate

Task: $ARGUMENTS

You orchestrate; procedure skills do the work and write the plan directory
(`.claude/skills/feature/plan-bus.md`). Pass the plan directory path, the
target branch and the verification scope to every subagent.

## 0. Task

Create `.claude/plans/<slug>/00-task.md`: responsibility, target (`master`
unless the user says `dev`), branch `feat/<slug>` or `refactor/<slug>`,
verification scope (scoped development checks; final integrated gate), success metric (crate exists with a card,
layer and tests; the code it replaces, if any, is gone from the old crate).

## 1. Design (architect)

Spawn `architect` to run, in order:

1. `placement` → `10-placement.md`. It must conclude **new crate** with at least
   two criteria met. If it concludes "module of an existing crate", stop and
   switch to `crate-extend`.
2. `port-design` for every capability the crate uses or provides.
3. `docs-adr` decides the documents: an ADR when the crate takes a
   responsibility from another crate or adds a port between layers; otherwise
   its card and layer-note entry.
4. `30-design.md` with: crate name `nocterm-<name>` in `crates/<name>`, layer,
   draft card, public surface, error type, dependencies (checked against the
   layer table), wiring step, tests per layer, ADR, commit plan.

Show `30-design.md` to the user and wait for approval (AGENTS.md gate).

## 2. Build (implementer)

Skeleton, in one step:

- `crates/<name>/Cargo.toml` like `crates/containers/Cargo.toml`: `edition`,
  `license-file` and `lints` from the workspace, `publish = false`, a
  `description`, and `[package.metadata.nocterm] layer = "<layer>"`.
- An entry `nocterm-<name> = { path = "crates/<name>" }` under
  `[workspace.dependencies]` in the root `Cargo.toml` (`crates/*` are members).
- `crates/<name>/src/lib.rs` starting with the card (`Owns:`, `Does not own:`, `Ports`,
  `Settings`, `Globals`, `Invariants`), modules named for what they own,
  `error.rs` (`typed-errors`).
- `test-support` fakes behind a feature, used only from dev-dependencies.

Then each behaviour with `behavior-tests` (cover), `wiring` if the crate is
installed by the app. Use `gate` scoped mode after coherent edits when needed;
record evidence/pending checks in `40-steps.md`, not a command bundle per step.
Run `cargo xtask docs` once the dependency graph changes have settled.

## 3. Verify (tester)

Use `gate` scoped mode and `behavior-tests` for gaps; reuse valid evidence.
Include dependency/notice checks for the new manifest at the final integrated
checkpoint. If notices are stale, regenerate them once inputs have settled
(`python3 scripts/generate-license-notices.py`).

## 4. Review (reviewer)

`design-review` → `60-review.md`. Blocking findings go back to the
implementer; after two failed rounds escalate.

## 5. Wrap up

The orchestrator assigns one full `gate` for the integrated task after review
fixes; creating this crate does not require a separate per-slice full run.
Report each actual gate command with its result, the review verdict, the baseline
delta, and proposed commits (`feat(<name>): …` / `refactor(<name>): …`, ADR as
`docs: …`). Commit only if the user asked.
