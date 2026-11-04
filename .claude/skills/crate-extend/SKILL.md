---
name: crate-extend
description: Add behavior to an existing crate, checking ownership, affected APIs and tests; refactor only where the design or hard limits require it.
argument-hint: "<change> [crate] [target: master|dev]"
---

# Extend a crate

Task: $ARGUMENTS

Follow AGENTS.md's Workflow and Definition of done. One agent and one compact
plan are sufficient for scoped work; use [the handoff contract](../feature/plan-bus.md)
when delegating. Record behavior, target/base, branch, affected crate/layer and checks.

1. Use `placement` to confirm the behavior fits the crate's ownership card and
   allowed dependencies. Separate responsibilities across layers where needed.
2. Inspect the files/types that will grow. Use `cargo xtask survey <package>`
   and `responsibility-map` when ownership is unclear. Early warnings are
   advisory: explain consequential growth, but extract only where responsibility
   or hard limits require it. Do not require a no-new-warnings metric.
3. Design new collaborators with `port-design`, fallible APIs with `typed-errors`,
   and installation/persistence changes with `wiring`; apply `docs-adr` as needed.
4. Implement with `behavior-tests`. If extraction is needed, characterize behavior
   first and use `owner-extract`; keep a cohesive fix on one branch where useful.
5. Use `gate` for affected behavior and consumers. Review with `design-review`,
   fix findings, then finish the applicable input-based completion checks.
6. Report behavior, checks/results, relevant boundary/count changes and remaining
   limitations. Commit/push/PR only when requested.
