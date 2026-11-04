# 0132. Agent procedures scale to impact and machine resources remain local

Status: accepted

## Context

[ADR 0059](0059-architecture-procedure-skills.md) added committed procedures,
shared plans and hooks to apply architecture rules consistently. Its resource
policy forced a fixed Cargo job count and one machine-wide lock. Its workflow
structure grew into mandatory roles, artifact files and repeated approval/check
steps even for scoped changes. Resource constants drifted between instructions,
implementation and tests; unrelated builds competed for a common queue.

Product invariants need consistent enforcement. Machine budgets and the amount
of process needed for a particular change depend on the contributor and task.

## Decision

This record supersedes ADR 0059's fixed Cargo budget/required wrapper and its
prescriptive orchestration and artifact structure. Its architecture rules,
committed procedure skills, role write boundaries and release/debt/hook-bypass
guards remain in force. Accepted historical records are not rewritten.

`AGENTS.md` owns current workflow and local verification policy. Scoped work may
use one agent and a compact ignored plan. Optional role profiles inherit caller
model choices; detailed maps/files and explicit handoff blocks are used when
helpful. A request to implement an existing plan supplies its authorization;
material unresolved decisions or changes to scope/authority still need the
user. Actionable findings are fixed rather than stopped by a round counter.
Architecture warning thresholds remain advisory; hard limits do not change.

Local completion checks follow changed inputs and affected consumers. Broad
shared changes, uncertain workspace impact or an explicit full-verification
request require the full workspace suite. Prose and tooling-only changes use
relevant reference, script and configuration checks. Required CI, architecture
limits, plugin feature/fixture coverage and release checks are unchanged.
Evidence is reused only while its relevant source/configuration remains valid.

Cargo keeps the caller's environment/configuration and default job selection.
Direct Cargo is allowed. The optional `cargo-serial.sh` helper defaults to a
lock in Git's common directory, coordinating linked worktrees independently of
other clones. `NOCTERM_CARGO_LOCK` selects an explicit lock path for a wider
group, or `off` to disable the helper lock. Relative paths are relative to the
invocation directory; an absolute path shares identity across directories.
Locking requires `flock`; if unavailable, the helper fails promptly and explains
how to install it, choose direct Cargo or explicitly disable its lock. There is
no directory-lock fallback that can survive a crashed owner indefinitely.

Vendor and application changes may share a PR while remaining separate commits
and following CONTRIBUTING.md's merge/history rules. Plugin procedures describe
the existing feature-gated user-plugin contract and preserve its trust boundary;
adding native visibility does not grant a guest capability.

## Alternatives

- Keep a fixed job count and per-user machine lock: encodes one machine budget
  and blocks unrelated clones. Contributors can explicitly choose both when needed.
- Add PID-based stale-lock recovery or a new cross-platform lock service: adds
  concurrency machinery for an optional development helper. Direct Cargo is the
  portable path when `flock` is unavailable.
- Require four roles, multiple plan files and full local workspace checks for
  every change: creates repeated handoffs and unrelated builds without matching
  the change's impact. Required CI remains the integration backstop.
- Remove architectural or release guards with procedural constraints: confuses
  product invariants with execution preferences; those guards are retained.

## Consequences

- Resource policy has no numeric constant to synchronize across prose and tests.
  Behavior tests cover inherited job settings and actual lock scope/lifecycle.
- Contributors choose local budgets and wider coordination where their machines
  need them. The optional helper reports unsupported locking explicitly.
- Procedures share one policy and one evidence record instead of creating
  another approval, refactor or full gate solely for a stage or warning.
- Completion reports identify their actual coverage; a scoped pass is never
  presented as proof that the whole workspace or every optional feature passed.
