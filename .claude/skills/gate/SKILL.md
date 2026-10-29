---
name: gate
description: Choose scoped checks during development and run one centralized final verification of the integrated task. Reuse valid results; record scope, failures and pending final checks in 50-gate.md. No fixed per-edit or per-step bundle.
user-invocable: false
---

# Verification

Follow AGENTS.md "Verification policy" and "Definition of done". Every Cargo
command uses `.claude/hooks/cargo-serial.sh` (jobs2 and the shared lock).
These are local agent-work rules; required CI checks still run independently.
In older plans, read "quick" as scoped mode, not as a fixed command bundle.

## Ownership and timing

- The orchestrator owns one queue and names the final-gate executor. Agents
  request checks with their scope and reason; do not start independent full
  pipelines or queue the same check while one is pending/running.
- Finish a coherent edit before checking its source. Stop editing that source
  and its manifests until the check finishes; unrelated work may continue.
- There is no required bundle after every edit, step, extraction, subagent,
  worktree or backlog slice. Use scoped checks during Build/Verify and fixes.
- Full mode belongs to completion of the integrated task, after review fixes
  and before Done. A partial handoff/review does not require it. Each changed
  repository in a multi-repository task needs its own final evidence.

## Scoped mode (choose by uncertainty)

These are alternatives, not a sequence to execute in full:

| Question/change | Smallest useful check |
| --- | --- |
| One reproduced regression or fixture fix | `cargo test -p <package> --lib <behavior-filter>` or the existing `--test <target> <filter>` |
| A coherent behavior change in one owner | Its affected test target; expand to package tests if the affected behavior spans that package |
| Compilation/API uncertainty | `cargo check -p <package> --lib` and affected consumers; omit if the selected test already compiles them |
| A lint failure | `cargo clippy -p <package> --lib -- -D warnings` or the affected test/binary target |
| Formatting | Format changed Rust files; defer the workspace format check to the final gate |
| Owner shape/early warning | Inspect the diff/counts; use `cargo xtask survey <package> --type <Type>` when needed |
| Skills/agent instructions only | `python3 scripts/check-claude-config.py`, links and stale references |
| Other prose only | Links and stale references; no Cargo unless executable/generator inputs also changed |

Prefix the Cargo examples above with `.claude/hooks/cargo-serial.sh`.
A test already builds its target: do not routinely run check, build, clippy and
all package tests for the same small edit. Full architecture scans, workspace
builds/tests, rustdoc, dependency and licensing audits are not per-step checks.
Expand to a consumer/feature/platform when an API, dependency/feature graph or
observed failure affects it; state the concrete reason rather than repeating a
standard bundle. Batch generated-reference updates after their inputs settle.

## Final mode (one integrated completion checkpoint)

The assigned executor runs the applicable AGENTS.md final checks:

```sh
W=.claude/hooks/cargo-serial.sh
$W cargo fmt --all --check
$W cargo build --workspace --all-targets
$W cargo clippy --workspace --all-targets -- -D warnings
$W cargo test --workspace
$W cargo xtask architecture
$W cargo xtask docs --check
```

Documentation/instruction-only tasks use scoped documentation checks instead.
Add the applicable SSH/hub, user-plugin, SDK/example/guest and vendor checks
specified in AGENTS.md and CONTRIBUTING.md. Do not run an optional matrix for
unchanged inputs. Where compatible, enable required live fixtures in the final
workspace test run rather than rebuilding the app with several package/feature
selections; confirm the gated tests actually executed. Do not count skipped
fixtures or zero matching tests as coverage.

| Additional changed inputs | Check at the relevant completion point |
| --- | --- |
| Dependencies/manifests | `cargo machete --skip-target-dir`; notice inputs/freshness as required by AGENTS.md |
| Public Rust documentation | `cargo doc -p <affected-package> --no-deps`; expand only for affected cross-crate docs |
| Hooks | `python3 scripts/test-claude-hooks.py` |
| Skills/agent instructions | `python3 scripts/check-claude-config.py` |

## Failures, reuse and caches

- A failure stops that check chain. Reproduce/fix the failing target first,
  then resume pending/invalidated checks; do not restart a pipeline at fmt/build
  automatically. Do not fix lint thresholds, baselines or hooks to get a pass.
- Keep a successful result when its relevant inputs/configuration are unchanged.
  Record the worktree/source snapshot (HEAD and relevant uncommitted/untracked
  changes), command, packages/targets/features and toolchain. A new stage or
  report is not a new source snapshot. Never count an aborted/queued command as
  executed or carry evidence across unrelated source/configurations.
- A reviewer reads existing evidence. It reruns only a missing, stale or
  specifically suspect check, not architecture/the whole suite by default.
- For a final-gate fix, identify affected evidence explicitly. A dependency,
  feature, shared API or cross-cutting change may invalidate broad coverage;
  a prose edit does not invalidate compiled behavior. Do not claim Done with
  required coverage still missing.
- Keep one stable target directory per worktree. Do not routinely touch source
  timestamps, clean caches or create another cold target. Investigate concrete
  provenance/cache failures and invalidate only what is necessary.
- Baseline delta must only decrease. Early warnings are advisory; inspect a
  touched type near its limit with `responsibility-map` before growing it.

## Output: `50-gate.md`

One shared record is enough; append new evidence instead of cloning full gate
reports for each agent. Use a compact table:

```text
Scope: scoped | final; worktree/source snapshot; toolchain/features
Owner/executor: <agent>
Command | Packages/targets | Result | Log
...
Reused: <valid prior results and why they still cover this state>
Pending final checks: <list | none>
Baseline delta / touched early warnings: <facts | not checked yet>
Not run: <check and input-based reason>
Timing, if available: queue wait / build / test execution
```

Report what actually ran. A scoped PASS is not a full-gate PASS; deferred final
checks are pending, not failures. End with the plan Result block; report a
failed prerequisite/check as blocked until fixed, without launching a new full
pipeline merely to change that status.
