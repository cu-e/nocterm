# Agent guide

Follow [CONTRIBUTING.md](CONTRIBUTING.md) for Git, PR and vendor rules.

## Hard rules

- Record the target (`master` release / `dev` integration); branch from it using
  `<type>/<slug>`. Never commit directly to `master` or `dev`.
- Commit, push and open PRs only when the user asks; use Conventional Commits.
- Never edit `CHANGELOG.md`, `version.txt`, `.release-please-manifest.json` or
  create tags: releases are automated. Never bypass hooks or weaken checks.
- Rust files stay at or under 800 lines, tests included. This is a ceiling, not
  a target; never add an entry to `xtask/oversized-files.toml`.
- Report actual verification and its limits. A structural check does not prove
  correct ownership, rendering or performance.

## Read first

Read [the layer table](docs/ARCHITECTURE.md#layers), the affected crate's `//!`
card and relevant sections of its layer note. Use the [ADR index](docs/adr/README.md)
to find current decisions for the changed contracts, including superseding records.
Expand reading when dependencies or ownership remain unclear; an incidental crate
mention does not require reading every historical ADR. Accepted ADRs are immutable.

## Where code goes

Name the crate/layer and responsibility before non-trivial changes:

1. Pure rules and formats: domain; reusable low-level primitives: foundation.
2. Real filesystem, network and process I/O behind contracts: adapters.
3. GPUI entities with background work and no views: services.
4. Views and presentation: features; shared components: ui. Composition: app.
5. Features meet through narrow consumer-named ports, never feature dependencies.

Reject a candidate that violates its layer or ownership card. Reuse a fitting
crate; propose a new one if none fits. Existing mixed responsibilities are debt.
For unclear placement or extraction use [placement](.claude/skills/placement/SKILL.md)
and [responsibility-map](.claude/skills/responsibility-map/SKILL.md).

## Design rules

1. Keep state within one responsibility. Before adding an owner, name its state,
   invariant, transitions and lifetime, including cancellation/late results.
   Do not add fields above 15 or spread a parent's impl into more files.
2. Stateful types gain no `pub`/`pub(crate)` fields. DTOs/settings may expose
   plain data; transitions preserve invariants and one authoritative state.
3. No new `impl Global`, reads of another crate's global, or `on_app_quit`.
   Inject collaborators explicitly; compose their lifecycle in bootstrap.
4. Fallible cross-crate APIs use typed errors (`thiserror`), preserving causes;
   convert to text only for display/logging.
5. No I/O in domain code or protocol-specific types (including ACP) in its APIs;
   no domain rules in views. Existing violations do not authorize growth.
6. No new `#[expect]`/`#[allow]` for `clippy::too_many_lines` or
   `clippy::cognitive_complexity`. `xtask/baseline.toml` may only shrink;
   lower entries when debt shrinks. See [guardrails](docs/architecture/guardrails.md).
7. Keep files focused and easy to read. Separate independent responsibilities
   when they emerge, even below warning thresholds. Extract state with its
   transitions, or cohesive pure functions; never split just by line count,
   invent empty owners, or fill a file to its limit. Shared types remain with
   their owner. Keep vendor and application changes in separate commits.

Warnings (640 lines, 12 fields, four impl files) invite inspection, not automatic
extraction. Compare plausible alternatives for consequential decisions: correctness,
reuse, runtime/memory cost and maintenance. Choose the simplest adequate solution;
avoid speculative frameworks and an exhaustive search for a theoretical optimum.

## Reuse before writing

Search for an existing capability and check its semantics before adding one:

- Persistence: `nocterm_store::{Persisted, ShutdownCoordinator}` and its keyed/append
  owners; lower-level ordering: `nocterm_core::persist::{WriteQueue, WriteGate, ShutdownDeadline}`.
- Private files: `nocterm_core::fs::{write_private_atomic, create_private, AtomicPublication}`;
  configuration: `nocterm_core::persist::{save, save_text, save_preserving}`.
  Check [symlink/publication semantics](docs/adr/0015-private-file-publication.md).
- Native files: `nocterm-session::local_fs::{LocalFs, LocalMutations}` via
  `nocterm-localfs`; programs: `ProgramOpener` via `nocterm-local::LocalProgramOpener`.
  `nocterm-local::fs` is not an API. Reuse shutdown, traversal and shared port fakes.

Use [port-design](.claude/skills/port-design/SKILL.md) for contracts and
[wiring](.claude/skills/wiring/SKILL.md) for retained installation/persistence.

## Performance

For changed hot paths, assess frequency × work per event and retained memory.
Keep blocking I/O and expensive computation off render/input callbacks. Remove
redundant work, repeated parsing/scans and unchanged-state notifications; use bounded,
coalesced background work with cancellation. Check the affected visible behavior.
Cache worthwhile repeated work with an explicit owner, complete key, invalidation,
memory bound and teardown; never use a cache as authorization or a second source
of truth. Follow [performance](.claude/skills/performance/SKILL.md) for rendering,
cache and cost evidence. Measure representative before/after cases for optimization
claims; describe unmeasured expectations honestly. Routine edits need no benchmark suite.

## Tests

Test through public behavior, actions, events and rendering, not private view/model
fields. Extend behavior-named tests; reuse/extend shared port fakes. Refactors keep
assertions and observable behavior; mechanical import/fixture moves are allowed.
Use [behavior-tests](.claude/skills/behavior-tests/SKILL.md) for coverage and examples.

## Documents

Keep `docs/` about the current system; plans, progress and audit trails go in ignored
`.claude/plans/`. Do not record session authorization/status in `docs/`.
Update current references when symbols move; accepted ADRs keep historical context.
Cross-cutting decisions get an ADR. New crates start with a short `//!` ownership
card; update existing cards when boundaries change. Details: [docs-adr](.claude/skills/docs-adr/SKILL.md).

## Workflow

Use one agent and one compact ignored plan for ordinary non-trivial work: goal,
branch/base, affected owners/layers, approach, risks and checks. Trivial edits need
no plan. Load only procedures relevant to the task; output filenames are optional.
Delegate independent work when complexity/risk warrants it; supply scope and evidence.
A request to implement a plan authorizes routine decisions within it. Ask only for
material unresolved choices or changes of scope/authority. Review committed,
staged, unstaged and relevant untracked task changes using [design-review](.claude/skills/design-review/SKILL.md).
Fix actionable findings; escalate concrete blockers or repeated lack of progress.
No blocking findings may remain at completion. Preserve unrelated work.

## Verification policy

Use [gate](.claude/skills/gate/SKILL.md) to select checks by changed inputs and
consumers. Check coherent edits, not each moved method; a test already compiles
its target. Reuse results while relevant source/configuration stays unchanged.
For parallel work, use one verification executor and do not edit checked inputs
while checks run. Keep stable Cargo targets; investigate before clearing caches.
Cargo uses local settings; the coordination helper is optional (CONTRIBUTING.md).

## Definition of done

After review fixes, finish the [completion matrix and applicable fixtures](.claude/skills/gate/SKILL.md#definition-of-done).
Broad shared changes, uncertain workspace impact or an explicit full-verification
request require the full suite there. Required CI and limits remain unchanged.
Report actual commands/results, scope and omitted checks with reasons; a scoped
pass is not a workspace pass. Commit, push and PR still require the user's request.
