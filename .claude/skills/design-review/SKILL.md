---
name: design-review
description: Review a change for architectural anti-patterns that checks cannot see — wrong layer, broad ports, facade owners, sliced impls, string errors, globals, private-field tests — and rank findings as blocking or advisory. Used by the reviewer agent and at the end of every workflow skill.
user-invocable: false
---

# Design review

Input: the task, recorded target and existing evidence. Inspect committed changes
with `git diff <target>...HEAD`, staged and unstaged changes with `git diff HEAD`,
and relevant untracked files from `git ls-files --others --exclude-standard`.
Read surrounding owners/consumers as needed. Record findings in the compact plan.

Automated checks cover limits and debt counts; this review covers intent.
Inspect existing scoped/final evidence and pending checks. Do not rerun full
architecture or tests just because review started; the `gate` skill governs
reuse and any missing/suspect check. Finish the applicable input-based Definition
of done after review fixes. Read relevant plan/placement/map notes when present
and check the code against the authorized task.

## Ownership and cost

For changed state or boundaries, trace one actual use case through its owners:

- Who owns the authoritative state, its invariant and all transitions? Does the
  proposed split reduce coupling, or just rearrange fields to satisfy a count?
- Who creates/releases subscriptions, tasks and resources? What happens on
  close, cancellation, account/session changes and delivery of a stale result?
- Is each module cohesive and understandable? Independent responsibilities need
  separation even below limits; small pure modules need no artificial owner.
- For affected hot paths, is work bounded and proportional to changed/visible
  data? Inspect redraw/notification fan-out and cache invalidation. Use
  `performance` for relevant runtime/GUI evidence; do not demand unrelated benchmarks.

Structural PASS is evidence about measured rules, not semantic approval. The
current public-field metric covers feature/service; impl spread is counted above
four files; ownership cards are checked for labels. Review the design rules even
where those checks do not report a violation.

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
| 16 | A touched responsibility near a hard limit whose growth has not been considered | advisory |
| 17 | A contribution bypassing its `PLUGIN`/`CORE` catalog or registration, a panel entity held outside the workspace, an eagerly created widget, or a native getter misrepresented as guest capability access | blocking; honor the explicit core/required-panel exceptions in `plugin-api` |
| 18 | Mirrored authoritative state, stale async delivery, missing cancellation/teardown or cache invalidation | blocking when a concrete incorrect transition or unbounded resource lifetime is possible |
| 19 | Blocking/expensive work added to render/input paths, redundant full refreshes, unbounded background work or caches | blocking for a demonstrated regression or an identifiable unbounded path; give the workload and consequence |

For each finding give file:line, the rule (AGENTS.md section or ADR), a triggering
scenario/consequence and the smallest fix. Distinguish evidence from hypotheses;
an unmeasured speedup is not a verified result. Style preferences alone are advisory.

## Review to record

```
# Review <date> against <target>
## Blocking
1. <file:line> — <anti-pattern #> — <consequence> — fix: <…>
## Advisory
## Checked and fine
Verdict: approve | changes requested
```

Fix actionable findings under AGENTS.md's Workflow. Escalate material unresolved
choices or concrete blockers, without a fixed round count.
