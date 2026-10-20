---
name: docs-adr
description: Keep architecture documents true after a change — decide whether an ADR is needed and write it, update crate cards, layer notes and generated references, and remove stale symbol references. Use when placement or a refactor marks "ADR needed", a crate boundary moves, or a symbol is renamed or removed.
user-invocable: false
---

# Documents and decisions

Rules: AGENTS.md "Documents". `docs/` describes the current system; plans and
progress stay in `.claude/plans/`.

## 1. Is an ADR needed?

Yes, when the change:

- splits or merges crates, or moves a responsibility between crates or layers
  (including a new crate that takes code from another);
- adds a port between layers or features, or changes who holds authority;
- adds a cross-cutting mechanism (persistence, shutdown, error, check);
- supersedes an accepted record.

No, for a new leaf crate for a new subject that adds no port between layers
(its card and layer-note entry suffice), a new module inside a crate's existing
`Owns:`, an owner extraction that keeps the crate boundary, or bug fixes.

## 2. Write it

File `docs/adr/NNNN-<slug>.md`, `NNNN` = highest existing number + 1:

```
# NNNN. <Decision as a sentence>

Status: accepted

## Context
<forces and the current problem, with file or crate names>

## Decision
<what is now true: owners, ports, invariants; link related ADRs>

## Alternatives
<each rejected option and why>

## Consequences
<what gets easier or harder; checks or debt that change>
```

Add a row to `docs/adr/README.md` (`| [NNNN](NNNN-slug.md) | <sentence> |`).
Accepted records are never edited; a later record supersedes them and says so.
No session status, dates of work, or "implemented by" notes.

## 3. Cards and notes

- The crate's `//!` card (`Owns:`, `Does not own:`, `Ports`, `Settings`,
  `Globals`, `Invariants`) must describe the code after the change. New crates
  need one; `cargo xtask architecture` fails without it.
- Update the layer note in `docs/architecture/<layer>.md` when a crate joins
  or leaves the layer or a port between layers changes.
- `cargo xtask docs` regenerates `docs/reference/*` and
  `docs/architecture/dependencies.md`; run it after changing settings, tokens,
  actions, keymap or internal Cargo dependencies.

## 4. Stale references

For each removed or renamed symbol, module or crate:

```sh
grep -rn "<old name>" docs/ AGENTS.md CONTRIBUTING.md .claude/skills .claude/agents
```

Update current documents; leave accepted ADRs as historical text.

End with the Result block listing documents written or changed.
