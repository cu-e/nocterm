---
name: gate
description: Select local checks by changed inputs, reuse valid evidence, and finish the applicable Definition of done without per-edit or per-stage command bundles.
user-invocable: false
---

# Verification

AGENTS.md's Verification policy and Definition of done are authoritative for
scope, completion checks, fixtures and result reuse. Required CI is independent.
Cargo resources follow CONTRIBUTING.md; the coordination helper is optional.

## Select by the uncertainty

| Question/change | Useful scoped check |
| --- | --- |
| Reproduced regression | One `cargo test -p <package> --lib <filter>` or existing integration target/filter |
| Behavior spans an owner/package | Affected test targets and consumers |
| API/compilation uncertainty | `cargo check -p <package> --lib` plus affected consumers, unless tests already compile them |
| Lint failure | Clippy for the affected package/targets/features |
| Rust formatting | Format changed sources; use `cargo fmt --all --check` for completion |
| Ownership/guarded structure | Inspect changed counts; survey when useful; run architecture for affected guardrails |
| Skills/agent instructions | Configuration validator, links and stale references |
| Hooks/scripts | Their behavior tests and language syntax/configuration checks |
| Prose only | Links/references; Cargo/generated checks only for affected generator inputs |

Finish a coherent edit before checking. Do not edit checked inputs while their
check runs. A test already compiles its target; do not routinely precede it with
both check and build. In parallel work, coordinate one executor and avoid
queuing the same pending check twice.

## Completion and failures

At integrated completion, choose the applicable AGENTS.md scope after review
fixes. Broad shared changes, uncertain workspace impact or a user request need
the full workspace suite listed there. A small change does not inherit that
suite merely because a stage or worktree ended. Apply required plugin-feature,
SSH/hub, guest, dependency/notice and vendor checks when their inputs change;
confirm fixtures actually ran, rather than counting skipped/zero tests.

Rerun a failing target first. Preserve successful evidence when relevant
source/configuration is unchanged; identify which results a fix invalidates.
A new stage/report does not invalidate evidence. Reviewers inspect it and rerun
only missing, stale or specifically suspect checks. Keep stable targets and
investigate concrete stale artifacts before cleaning or touching all sources.
Do not change checks, architecture limits or baselines to obtain a pass.

## Evidence

One compact record in the plan or a shared `50-gate.md` is enough:

```text
Command | Source/configuration and targets/features | Result | Log if needed
Reused: <prior evidence and why it still applies>
Pending/unavailable: <check, reason and scope limitation>
```

Record the worktree/source snapshot, including relevant uncommitted/untracked
inputs. Separate queue wait and execution time when available. Report scoped
coverage as scoped; a completion pass does not assert a full-workspace pass.
