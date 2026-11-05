---
name: feature
description: Carry a feature or non-trivial change through design, implementation, verification and review, scaling delegation and planning to the task. Use for end-to-end work or /feature.
---

Task: $ARGUMENTS

Follow AGENTS.md's Workflow and Definition of done. Use one agent for scoped work;
delegate distinct responsibilities when complexity, risk or independent work
benefits from it. The role files in `.claude/agents/` are optional and inherit
caller model settings. Never assume a delegate has unseen context: supply task,
branch/base, plan/evidence location, scope and relevant prior findings.

1. Record a compact goal, approach, affected crates/layers, risks and check scope.
   Use [plan-bus.md](plan-bus.md) for optional structured handoffs. Honor an
   existing authorized plan; resolve only material unanswered choices.
   For changed ownership, name the invariant, transitions and lifetime; for
   consequential alternatives, compare correctness, reuse, cost and maintenance.
2. Select relevant procedures rather than launching every workflow:

   | Need | Procedure/workflow |
   | --- | --- |
   | Responsibility with no fitting crate | `crate-new` |
   | Behavior inside an existing crate | `crate-extend` |
   | Restructure within a crate | `refactor` |
   | Move ownership across crates | `crate-split` |
   | Shrink a debt inventory entry | `debt-paydown` |
   | Rendering, caches or other hot paths | `performance` |

   A cohesive change may combine them on one branch; an early warning alone
   does not require a refactor workflow. Split independently reviewable work
   when useful, respecting the user's requested base/branch and existing work.
3. Implement the authorized design with behavior tests. Use `gate` scoped mode
   after coherent edits where a check answers a concrete uncertainty.
4. Review committed, uncommitted and relevant untracked task changes against the
   recorded target. Use `design-review`; fix actionable findings and verify
   affected inputs. Escalation follows AGENTS.md, without a fixed round count.
5. Assign one completion executor when working in parallel. Finish the
   input-based Definition of done after review fixes; preserve valid evidence.
6. Report what changed, commands/results, scope and remaining limitations.
   Propose commit/PR text only when useful. Vendor and app changes belong in
   separate commits; commit, push and PR actions require the user's request.
