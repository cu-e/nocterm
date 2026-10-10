---
name: feature
description: Run the full agent pipeline (architect → implementer → tester → reviewer) for a feature or non-trivial change. Use when the user asks to build a feature end to end or invokes /feature.
---

Drive the pipeline described in AGENTS.md for the task: $ARGUMENTS

You are the orchestrator. Subagents start with no context, so every prompt must
carry the task, plan, branch name, target/base branch, verification scope and prior
findings, plus paths to AGENTS.md, CONTRIBUTING.md, docs/ARCHITECTURE.md, relevant
layer notes and docs/adr/README.md. Plans and reports go in ignored `.claude/plans/`.

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
   delta in the proposed PR description; until that file exists, report manual
   metric changes and the absence of ratchets. Keep vendor changes separate from
   application changes. Commit, push and open the PR only if the user asks.

Documentation-only changes skip subagents under AGENTS.md and use its documented
verification scope.
