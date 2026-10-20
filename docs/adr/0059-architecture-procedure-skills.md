# 0059. Architecture rules are applied through committed procedure skills and guards

Status: accepted

## Context

AGENTS.md states where code goes and which shapes are forbidden, and
`cargo xtask architecture` fails when a limit is crossed ([ADR0012](0012-architecture-debt-ratchets.md)).
Neither says how to get there. Agents meeting the 800-line or 15-field limit
moved methods of the same type into new files, because the check reports
size, not responsibility. Decisions made by one pipeline stage reached the
next only through chat summaries. Several agents compiling at once exhausted
memory despite the `CARGO_BUILD_JOBS=2` rule, and crate ownership was
documented in some crates and not others.

## Decision

Procedures are Claude Code skills committed in `.claude/skills/`:

- Procedure skills, hidden from the command menu, each perform one step:
  `placement`, `responsibility-map`, `owner-extract`, `port-design`,
  `typed-errors`, `wiring`, `behavior-tests`, `docs-adr`, `gate` and
  `design-review`. Pipeline agents preload the ones their stage uses.
- Workflow skills, invoked by the user or routed by `/feature`, compose them:
  `crate-new`, `crate-extend`, `refactor`, `crate-split` and `debt-paydown`.
  Each keeps the pipeline gates; a layer conflict escalates to the user
  instead of being resolved by the agent.
- Stages communicate through an ignored plan directory,
  `.claude/plans/<slug>/`, with one file per step and a common result block
  (`.claude/skills/feature/plan-bus.md`).

Deterministic guards are committed hooks in `.claude/settings.json`, shared
by every checkout and worktree:

- before edits: release files are never edited; debt inventories only
  shrink; plain-data exemptions ask the user; architect and reviewer write
  only plan files and the tester asks before changing production code;
- before shell commands: no hook bypass; `master` and `dev` change only
  through pull requests; tags are left to release automation; no shell
  rewrites of protected files; compiling Cargo commands run through
  `.claude/hooks/cargo-serial.sh`, which forces `CARGO_BUILD_JOBS=2` and holds
  one lock per user on the machine for the command's duration only;
- after edits: a file near the line limit, a new file of foreign `impl`
  blocks, a catch-all module name or a new crate manifest adds a pointer to
  the matching skill.

`cargo xtask` gains structural signals that the skills consume: early
warnings at about four fifths of each limit, `cargo xtask survey` for file,
type, module-reference and method × field facts, and a required ownership
card in every crate. Hook and configuration behaviour is tested by
`scripts/test-claude-hooks.py` and `scripts/check-claude-config.py` in CI.

Hooks are guardrails for agents; CI and `cargo xtask architecture` remain
the authoritative gate.

## Alternatives

- Local hook settings: each contributor would hold a different copy, and
  worktrees would run without guards.
- Longer AGENTS.md rules: the rules already exist; what was missing was the
  procedure and the facts to apply them, and every added rule costs context in
  every session.
- Workflow scripts for every change: deterministic but expensive and
  non-interactive for decisions that need the user, such as layer conflicts.
- Failing at the early-warning thresholds: equivalent to lowering the limits,
  forcing splits before a design exists.

## Consequences

- A split starts from survey facts and a named owner, so moving methods of
  the same type between files is visible as a non-extraction in review.
- Cargo runs from agents queue instead of competing for memory; a Cargo
  command typed without the wrapper is refused with the rewritten command.
- Skills and agents cite repository paths and each other; the configuration
  check fails when a cited path, link, preloaded skill or hook script is
  missing, so renames must update them.
- New crates cannot merge without an ownership card.
