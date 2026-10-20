---
name: feature
description: Run the full agent pipeline (architect → implementer → tester → reviewer) for a feature or non-trivial change. Use when the user asks to build a feature end to end or invokes /feature.
---

Drive the pipeline described in AGENTS.md for the task: $ARGUMENTS

You are the orchestrator. Subagents start with no context, so every prompt must
carry the task, plan directory, branch name, target/base branch, verification scope
and prior findings, plus paths to docs/ARCHITECTURE.md, relevant layer notes and
docs/adr/README.md (AGENTS.md is loaded automatically). Decisions live in
`.claude/plans/<slug>/` as described in [plan-bus.md](plan-bus.md); every stage
reads the files earlier stages wrote and ends with its Result block.

0. **Route** — classify the task and follow the matching workflow skill instead
   of the generic stages below when one fits:

   | Task | Workflow |
   | --- | --- |
   | responsibility with no fitting crate, or "new crate" | `crate-new` |
   | behaviour added inside one crate | `crate-extend` |
   | restructure inside a crate, early warning or limit hit | `refactor` |
   | move a module or subject to another crate | `crate-split` |
   | shrink an `xtask/baseline.toml` or oversized-file entry | `debt-paydown` |

   Spanning several rows: run them as separate branches in the order
   refactor → split → extend. None fits (cross-cutting, app-only): continue
   with the stages below, using the procedure skills each agent preloads.

1. **Design** — spawn `architect` with the task and reading paths above. Require
   placement, shape, public surface, reuse, typed errors, docs/ADR and the target
   branch in the plan. Show it to the user and wait for approval; an explicit
   instruction to implement an already supplied plan supplies that approval.
   Relay unresolved design questions.
2. **Branch** — create `<type>/<slug>` from the recorded target: `master` for
   release-ready changes or `dev` for integration, per CONTRIBUTING.md. Verify
   the local base against its remote; do not reset or overwrite unrelated work.
3. **Build** — spawn `implementer` with the full approved plan.
4. **Verify** — spawn `tester` with the plan and the implementer's report. If it reports defects or red checks, send them to `implementer` and verify again.
5. **Review** — spawn `reviewer` with the plan and target. Include committed,
   uncommitted and untracked task changes; review must finish before requesting
   commit authorization. Require the blocking design checklist. Send `blocking`
   and `should-fix` findings to `implementer`, then re-run Verify and Review.
6. **Stop conditions** — after two failed Verify or Review rounds, stop and escalate to the user with the outstanding findings.
7. **Wrap up** — summarise what was built, each applicable verification command
   and result, review verdict and remaining nits. Propose Conventional Commit
   headers and a PR title. Include the target branch and the `xtask/baseline.toml`
   delta in the proposed PR description. Keep vendor changes separate from
   application changes. Commit, push and open the PR only if the user asks.

Documentation-only changes skip subagents under AGENTS.md and use its documented
verification scope.
