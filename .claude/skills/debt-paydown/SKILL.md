---
name: debt-paydown
description: Workflow for shrinking one entry of the architecture debt inventory (`xtask/baseline.toml`, `xtask/oversized-files.toml`) — pick the entry, find its cause, fix it with the matching procedure, and lower the recorded value. Use when the user asks to pay down debt or names a baseline entry.
argument-hint: "[baseline entry | package] [target: master|dev]"
---

# Workflow: pay down architecture debt

Entry: $ARGUMENTS

The inventory may only shrink (ADR 0012). Pass the plan directory, target
branch and verification scope to every subagent
(`.claude/skills/feature/plan-bus.md`).

## 0. Pick

If no entry is given, list candidates from `xtask/baseline.toml` and
`xtask/oversized-files.toml` with `cargo xtask survey <package>` facts and
propose the cheapest one with the most value (small counts first). Record the
entry, its current value and the target value in `00-task.md`; branch
`refactor/<slug>`.

## 1. Route by kind

| Section | Procedure |
| --- | --- |
| `struct_fields`, `impl_spread`, oversized files | `refactor` workflow (`responsibility-map` → `owner-extract`) |
| `pub_fields` | `owner-extract` (transition methods replace the field); a `[dto]` exemption in `xtask/architecture-policy.toml` only for plain data, and only after the user agrees |
| `string_errors` | `typed-errors` |
| `globals`, `global_reads`, `quit_handlers` | `wiring` (explicit dependencies, retained installation, shutdown coordinator) |
| `domain_io`, `domain_dependencies` | `port-design` (contract in the domain, implementation in an adapter) |
| `complexity_expects` | split the function by the decisions it makes, or extract an owner; never move the `#[expect]` |

Follow the chosen workflow's design approval, build, verify and review
stages.

## 2. Lower the record

In the same change, lower the entry in `xtask/baseline.toml` (or delete the
line in `xtask/oversized-files.toml` once the file is ≤ 800 lines). The
architecture check fails until the record matches; the project hook rejects
raising or adding entries.

## 3. Wrap up

Report the entry before/after, gate results and verdict. Commit as
`refactor(<scope>): …`; only if the user asked.
