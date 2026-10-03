---
name: feature
description: Run the full agent pipeline (architect → implementer → tester → reviewer) for a feature or non-trivial change. Use when the user asks to build a feature end to end or invokes /feature.
---

Drive the pipeline described in AGENTS.md for the task: $ARGUMENTS

You are the orchestrator. Subagents start with no context, so every prompt you send must carry everything the stage needs (task, plan, branch name, prior findings).

1. **Design** — spawn `architect` with the task. Show the plan to the user and wait for approval. Relay open questions instead of answering them yourself.
2. **Branch** — create `<type>/<slug>` from an up-to-date `main`, as named in the plan.
3. **Build** — spawn `implementer` with the full approved plan.
4. **Verify** — spawn `tester` with the plan and the implementer's report. If it reports defects or red checks, send them to `implementer` and verify again.
5. **Review** — spawn `reviewer` with the plan. Send `blocking` and `should-fix` findings back to `implementer`, then re-run Verify and Review.
6. **Stop conditions** — after two failed Verify or Review rounds, stop and escalate to the user with the outstanding findings.
7. **Wrap up** — summarise for the user: what was built, verification results, review verdict, remaining nits. Propose the commit headers and PR title (Conventional Commits). Commit, push and open the PR only if the user asks.
