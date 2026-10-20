---
name: crate-split
description: Workflow for moving a module or responsibility out of a crate into a new or existing crate — survey module references, decide the target layer per piece, escalate layer conflicts, move behind characterization tests, update cards and record an ADR. Use when a crate owns several subjects or a module's dependencies belong to another layer.
argument-hint: "<package>[::<module>] [target: master|dev]"
---

# Workflow: split a crate

Subject: $ARGUMENTS

Crate boundaries change, so this workflow always ends with an ADR. Pass the
plan directory, target branch and verification scope to every subagent
(`.claude/skills/feature/plan-bus.md`).

## 0. Task

`00-task.md`: subject, target, branch `refactor/<slug>`, metric (for example
"`nocterm-agent` lines −N; new crate `nocterm-<x>` owns `<module>`; no
`crate::<module>` references left in the old crate").

## 1. Survey and placement (architect)

1. `responsibility-map` on the crate. Use the `modules` section: a candidate
   module has most references to one or two workspace crates and few to its
   siblings (`crate::…`).
2. For each candidate run `placement`, using the references as the dependency
   list. Every dependency must be allowed by the candidate's layer.

### Layer conflicts: stop and escalate

When a candidate's dependencies span layers, it is not one responsibility yet.
Example from `nocterm-agent` (feature): its `thread` module references
`nocterm_ai` (domain) 162 times and `nocterm_workspace` (ui) 62 times, and
holds GPUI state with background work. As a service crate it could not depend
on `nocterm-workspace`; as a feature crate it would be a second agent feature
that the first must not depend on. Escalate with the options instead of
choosing:

- split the module first: the domain part (transcript rules, `nocterm_ai`
  types) moves down; the workspace-facing part stays in the feature;
- move the narrow contract it needs from `nocterm-workspace` into a lower
  crate (needs its own ADR);
- keep the module and run `refactor` inside the crate instead.

Write the conflict and options to `10-placement.md` and return `escalate`.

3. `port-design` for each reference that crosses the new boundary in both
   directions; reject a cycle (the old crate and the new one must not depend on
   each other through the same API).
4. `docs-adr`: draft the ADR (context, decision, alternatives) in
   `30-design.md`; it is written to `docs/adr/` in the build stage.
5. `30-design.md`: target crate (new via the `crate-new` skeleton, or existing),
   files moved, public surface, ports, wiring change, step order, commits.

Show the design to the user and wait for approval.

## 2. Move (implementer)

1. `behavior-tests` characterize through the old crate's public API.
2. Create the target crate (`crate-new` skeleton) or a module in the existing
   one; move code in compiling steps; re-export nothing from the old crate as
   a compatibility shim unless the design says so, with a removal step.
3. Move tests with the code they cover; the characterization tests stay green.
4. Update both cards, the layer note, `cargo xtask docs`, and write the ADR.
5. `gate` quick after each step; `40-steps.md`.

## 3. Verify (tester)

`gate` full plus `cargo machete --skip-target-dir` and the license-notice
check. Rerun the survey on both crates against the metric.

## 4. Review (reviewer) — `design-review`; two-round limit.

## 5. Wrap up

Report metric before/after, gate results, ADR number, baseline delta, verdict
and proposed commits (crate creation, each move, ADR). Commit only if the user
asked.
