# Plans and handoffs

Use one compact document under ignored `.claude/plans/<slug>/` by default.
Record the goal, branch/base, scope, approach, material decisions and risks,
checks/results and remaining work. Scale detail to the task. No plan files are
required for trivial edits; AGENTS.md governs authorization and completion.

Delegates receive the task and relevant context or a plan location explicitly.
Do not assume either that every tool shares history or that every delegate
starts without it. Record durable decisions that the next owner needs; avoid
copying the same evidence into several reports.

For larger work, the following are optional sections or separate files:

| Section / suggested file | Useful content |
| --- | --- |
| Task / `00-task.md` | goal, branch/base, scope, behavior or migration metric |
| Placement / `10-placement.md` | crate/layer choice and boundary rationale |
| Map / `20-map.md` | survey facts and candidate owners for a structural change |
| Design / `30-design.md` | approach, public surface, errors, tests, docs/ADR |
| Progress / `40-steps.md` | meaningful steps and deviations |
| Evidence / `50-gate.md` | command, source/configuration, scope, result/log, pending checks |
| Review / `60-review.md` | ranked findings and verdict |

Use only sections relevant to the task. A stage is complete when its work and
needed evidence are available, not when a file list has been filled. Revise a
changed decision with a short reason; an append-only transcript is unnecessary.
Store plans/progress here rather than in current-system `docs/`.

A handoff may use this optional compact block (a normal concise report is fine):

```text
status: done | blocked | needs-decision
changed: <files or sections>
evidence: <results and scope>
remaining: <next action or material unresolved choice>
```

All procedure-skill output filenames and Result blocks are suggestions under
this contract; their content may live in the shared compact plan. A new ADR or
routine implementation detail does not itself require another user approval.
Never proceed with a genuinely required unresolved authorization; follow
AGENTS.md for material decisions and concrete blockers.
