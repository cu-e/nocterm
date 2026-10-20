# Plan directory contract

Subagents start without context, so every workflow keeps its decisions in one
ignored directory, `.claude/plans/<slug>/`, and each step reads what earlier
steps wrote. Chat summaries are not a substitute: if a decision is not in the
directory, the next step does not know it.

| File | Written by | Contents |
| --- | --- | --- |
| `00-task.md` | workflow | task, workflow name, target branch (`master`/`dev`), feature branch, verification scope, **success metric** (from `cargo xtask survey`, e.g. "`Thread` fields 15 → ≤ 9; files ≥ 640 lines in `thread/`: 0") |
| `10-placement.md` | `placement` | crate, layer, reason, card check, new-crate decision |
| `20-map.md` | `responsibility-map` | survey facts, matrix interpretation, candidate owners with draft cards |
| `30-design.md` | architect / workflow | owners, ports, errors, wiring, tests, docs/ADR, commit plan; approved by the user |
| `40-steps.md` | implementer | ordered steps with `- [ ]` / `- [x]`, one transformation each, gate result per step |
| `50-gate.md` | `gate` | each command, its result, baseline delta, early warnings before/after |
| `60-review.md` | `design-review` / reviewer | findings (`file:line`, severity, scenario, fix) and verdict |

Rules:

- Create the directory in the first step; never write plans into `docs/`.
- Append, do not rewrite, earlier decisions; a changed decision records why.
- A step is complete only when its file exists with the sections above filled.

## Result block

Every procedure skill ends its turn with this block, and the workflow checks it
before the next step:

```
## Result
status: done | blocked | escalate
wrote: <files in the plan directory, and source files if any>
next: <next skill, or "return to workflow">
escalation: <question for the user, or "none">
```

`blocked` means a check or precondition failed and the step can be retried after
a fix; `escalate` means the decision belongs to the user (a layer conflict, a
new ADR, a rule that cannot be met). Never continue past `escalate`.
