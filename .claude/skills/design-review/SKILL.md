---
name: design-review
description: Review a change for architectural anti-patterns that checks cannot see — wrong layer, broad ports, facade owners, sliced impls, string errors, globals, private-field tests — and rank findings as blocking or advisory. Used by the reviewer agent and at the end of every workflow skill.
user-invocable: false
---

# Design review

Input: the diff against the recorded target, including uncommitted changes
(`git diff <target>...` plus `git diff` and untracked files), and the plan
directory. Output: `60-review.md`.

Automated checks already cover limits and debt counts; this review covers
intent. Read the plan's `10-placement.md`, `20-map.md` and `30-design.md`
first and check the code against them.

## Anti-patterns

| # | Look for | Blocking when |
| --- | --- | --- |
| 1 | Code in a crate whose card lists it under `Does not own:` or whose layer forbids its dependencies | always |
| 2 | A feature importing another feature, or a method added to an existing broad trait for a new consumer | always |
| 3 | New `impl Global`, `cx.global` of another crate, `on_app_quit` | always |
| 4 | New file containing only `impl Parent { … }` moved from the parent | always (not an extraction) |
| 5 | Owner taking `&mut Parent` / `Context<Parent>`, or one-field pass-through owners | always |
| 6 | `pub`/`pub(crate)` fields on a stateful type; getters that expose an owner's internals | always |
| 7 | `Result<_, String>` or `map_err(to_string)` across a crate boundary; lost `#[source]` | always |
| 8 | I/O (`std::fs`, `std::process`, sockets) in domain or protocol-specific types in domain APIs | always |
| 9 | Hand-written persistence instead of the store / `nocterm_core::persist` / `nocterm_core::fs` helpers | always |
| 10 | Tests reading private fields, or named after review rounds | always |
| 11 | Module named `utils`, `helpers`, `common`, `misc`, `part2` | advisory unless new crate |
| 12 | A duplicated port fake (instead of extending the shared one or proposing a shared home) | always |
| 13 | A duplicated non-fake test helper (for example a `click` wrapper) | advisory |
| 14 | Crate card, layer note or ADR not updated for a boundary change | blocking for boundary changes |
| 15 | `#[allow]`/`#[expect]` for `too_many_lines`/`cognitive_complexity`; raised baseline; new oversized entry | always |
| 16 | A file or type that crossed an early warning in this change with no map or split plan | advisory |

For each finding give: file:line, the rule (AGENTS.md section or ADR), the
concrete consequence, and the smallest fix.

## Output: `60-review.md`

```
# Review <date> against <target>
## Blocking
1. <file:line> — <anti-pattern #> — <consequence> — fix: <…>
## Advisory
## Checked and fine
Verdict: approve | changes requested
```

Changes requested go back to the implementer; after two failed rounds escalate
to the user (AGENTS.md "Pipeline").
