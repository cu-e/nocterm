# Agent guide

Instructions for AI agents working in this repository. Humans: see [CONTRIBUTING.md](CONTRIBUTING.md).

## Hard rules

- Follow [CONTRIBUTING.md](CONTRIBUTING.md): Conventional Commits, `<type>/<slug>` branches, never commit to `master` or `dev` directly. Record the target (`master` release / `dev` integration) and branch from it.
- Never edit `CHANGELOG.md`, `version.txt`, `.release-please-manifest.json` or create tags — releases are automated ([docs/RELEASING.md](docs/RELEASING.md)).
- Never bypass hooks (`--no-verify`) or weaken a check to make it pass.
- Commit, push and open PRs only when the user asks.
- Report verification honestly: what was run, what passed, what wasn't checked.
- Keep Rust files at or under 800 lines ([CONTRIBUTING.md](CONTRIBUTING.md#code-size)): split a
  growing file into child modules by responsibility instead of adding to it. Never add an
  entry to `xtask/oversized-files.toml`.

## Read first

Before designing or changing code, read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md),
the layer note of each touched crate, the [ADR index](docs/adr/README.md) and relevant
accepted decisions, and the crate's `//!` documentation in `lib.rs`.
Accepted ADRs are immutable; supersede a decision with a new ADR.

## Where code goes

Name the crate and layer in the plan, in this order:

1. Rules and data formats without I/O belong in domain crates.
2. Real filesystem, network and process I/O behind contracts belongs in adapters.
3. GPUI entities with background work and no views belong in service crates.
4. Views and presentation state belong in feature crates.
5. Features communicate through narrow workspace ports named for the consumer's
   need; do not expand a broad trait for another feature pair.

If the right crate does not exist, propose it instead of using the nearest crate.
Existing mixed responsibilities are migration debt, not a pattern for new code.
Use `Composer` and `SessionState` ([ADR 0009](docs/adr/0009-thread-composer.md),
[ADR 0010](docs/adr/0010-session-lifecycle.md)), `containers` / `containers-ui`, and
`src/bootstrap.rs` as examples of ownership and composition.

## Reuse before writing

- Ordered persistence: `nocterm_store::{Persisted, ShutdownCoordinator}`
  ([ADR 0013](docs/adr/0013-persistence-shutdown-service.md)); the common service
  uses `nocterm_core::persist::{WriteQueue, WriteGate, ShutdownDeadline}`
  ([ADR 0004](docs/adr/0004-write-queue.md)).
- Atomic files: `nocterm_core::fs::{write_private_atomic, create_private,
AtomicPublication}` for private data; `nocterm_core::persist::{save, save_text,
save_preserving}` for configuration. Check their symlink and format semantics
  before reuse ([ADR 0015](docs/adr/0015-private-file-publication.md)).
- Shutdown, link-safe filesystem traversal and test fakes: inspect existing
  implementations before adding one; propose shared extraction for missing reuse.

Layout, settings, snippets, connection state, Runtime chat/favorites and output
recording use the shared persistence service; keyed documents and append streams
share its driver. Native filesystem capabilities are
`nocterm-session::local_fs::{LocalFs, LocalMutations}`, implemented by the injected
`nocterm-localfs` adapter. Session `test-support` supplies shared in-memory port
fakes for dev-dependencies. `ProgramOpener` requests external local programs; the
injected `nocterm-local::LocalProgramOpener` owns their bounded native launch/wait
lifecycle. `nocterm-local::fs` is not an API.

## Design rules

These rules block review. `cargo xtask architecture` checks layers, file size,
complexity attributes and the exact debt inventory: field counts, public state,
impl spread, globals and panicking reads, domain I/O, quit handlers and string
errors. See [architecture guardrails](docs/architecture/guardrails.md) and
[ADR 0012](docs/adr/0012-architecture-debt-ratchets.md). Entries in
`xtask/baseline.toml` may only decrease; update them when debt shrinks.

1. Add state only within a type's single responsibility. Do not add a field when
   the resulting count exceeds 15 or create a type over that limit; extract an owner with
   private fields and transition methods. Do not spread its impl across more files.
2. Stateful types gain no `pub` or `pub(crate)` fields. DTOs and settings may expose
   plain data; methods preserve stateful invariants.
3. No new `impl Global`, reads of another crate's global, or `on_app_quit` handlers.
   Pass collaborators through explicit dependencies and constructors; compose
   them in bootstrap. Existing globals do not authorize new service lookup.
4. Fallible APIs crossing crate boundaries use typed errors (`thiserror`);
   convert them to `String` at display only.
5. Add no I/O to domain code or protocol-specific types (including ACP) to its APIs, and no
   domain rules to views. Existing violations do not authorize growth.
6. Never add `#[expect]` or `#[allow]` for `clippy::too_many_lines` or
   `clippy::cognitive_complexity`, and never raise a baseline to pass a check.
7. Extract responsibilities into modules that own types, not extra methods of a
   parent type. Keep vendor and application changes in separate commits; one PR
   may contain both under the merge rules in CONTRIBUTING.md.

## Tests

- Test behavior through actions, events, rendering and public methods; do not
  read or write private view/model fields to reproduce a regression.
- Name tests and files after behavior, and extend its existing test file. Do not
  create files named after review rounds (`audit`, `verification`, `round2`, etc.).
- Reuse existing port fakes and extend their capabilities. If they cannot be
  shared yet, propose a shared home instead of duplicating another fake.

## Documents

- `docs/` describes the current system. Plans, progress, session logs and audit
  trails belong in ignored `.claude/plans/` or the PR description.
- Never record session authorization or implementation status in `docs/`.
- When removing or renaming a symbol, search `docs/` and update current references;
  accepted ADRs retain historical context.
- Cross-cutting decisions get an ADR. Architecture notes describe layers, ports
  and invariants; product instructions belong in user documentation.
- For new crates, start `lib.rs` with a short `//!` card: purpose, `Owns:`,
  `Does not own:`, ports used/provided, settings section, globals and invariants.
  Describe actual boundaries; existing crates gain cards when their boundaries change.

## Workflow

Scale the process to the task. One agent may design, implement, verify and review
an ordinary scoped change. For large or risky work, delegate independent parts
or use the optional roles in `.claude/agents/`; use only the roles and models the
available tools and task need. `/feature` and the workflow skills are procedures,
not a requirement to launch four agents.

For non-trivial work, record the goal, affected crates/layers, approach, risks
and verification scope in a compact ignored plan. One document is enough;
[the plan contract](.claude/skills/feature/plan-bus.md) offers optional detail for
complex handoffs. Pass task, branch/base, scope and relevant evidence to delegates.
Review committed, uncommitted and relevant untracked changes.

A request to implement a supplied plan or proposed fix authorizes that work.
Resolve routine implementation details within that scope. Ask when a material
choice remains unresolved, scope or authority must change, or the user reserved
the decision. Continue fixing actionable findings; escalate a concrete blocker
or repeated lack of progress, rather than stopping at a fixed round count.
No blocking findings may remain at completion.

Architecture early warnings (640 lines, 12 fields, four implementation files)
are advisory. Inspect the responsibility and explain consequential growth;
refactor when the design needs it or a hard limit would be crossed. A warning
alone does not require an extraction, new branch or separate workflow.

## Verification policy

Choose local checks by this task's changed inputs and affected consumers;
[the gate procedure](.claude/skills/gate/SKILL.md) gives selection examples.
Required CI, lint thresholds, architecture limits and release gates are unchanged.

During implementation, finish a coherent edit, then run the smallest useful
check for the uncertainty. A test compiles its target; do not routinely precede
it with check and build. At completion, finish the applicable Definition of done
after review fixes. A handoff, extracted owner or backlog slice is not another
completion gate. Unrelated pre-existing changes are outside the task's evidence;
report that scope explicitly.

For parallel work, name one verification executor and coordinate checks on shared
sources or targets. Do not edit checked inputs while their check runs. Preserve
PASS evidence whose relevant source/configuration is unchanged; rerun a failing
target first and invalidate only affected results. Record command, scope,
source/configuration, result and any log once; do not rerun for another report.
Use stable Cargo targets; investigate stale artifacts before touching sources,
cleaning caches or creating cold targets.

Cargo uses the caller's environment and configuration. Set a local job budget or
coordinate builds when the machine needs it. The optional
[Cargo coordination helper](CONTRIBUTING.md#cargo-resources) preserves those
settings and can share a lock between worktrees or an explicitly selected group.
Machine resource preferences are not shell-hook denials.

## Definition of done

Run the checks applicable to the integrated change, and report each actual
command and result plus omitted/unavailable checks and why:

| Changed inputs / impact | Local completion checks |
| --- | --- |
| Documentation or agent instructions | Links and stale references; `python3 scripts/check-claude-config.py` for skills/agent configuration |
| Hooks or repository scripts only | Relevant script tests and syntax/configuration validators; no Cargo unless Rust or generator inputs are affected |
| Scoped Rust behavior | Formatting, lint and behavior tests for affected targets and consumers; architecture checks when guarded structure/ownership changes; generated references when their inputs change |
| Broad shared APIs, dependency/feature graph changes affecting many consumers, cross-cutting behavior, or uncertain workspace impact | Full workspace suite below, plus affected feature/platform fixtures |
| Explicit request for full verification | Full workspace suite and relevant additional checks |

The full workspace suite is `cargo fmt --all --check`,
`cargo build --workspace --all-targets`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`cargo xtask architecture` and `cargo xtask docs --check`. A scoped result must
not be presented as a full-workspace pass. Prose/tooling-only edits do not
require Cargo, generated-doc checks or product runtime tests when their inputs
are unchanged.

Additional checks follow actual changed inputs:

- SSH/SFTP changes run the required SSH fixture; hub adapter changes run the
  hub end-to-end test (see Commands).
- User-plugin code (`nocterm-plugin-*`, `plugins-ui`,
  `src/shell_setup/user_plugin*`) also runs workspace clippy and tests with
  `--features nocterm/user-plugins`; the default build compiles it out.
  Plugin SDK/example changes also build/test the example and run the gated
  WASM guest test. Preserve API/ABI compatibility checks for contract changes.
- Dependencies/manifests require applicable unused-dependency and notice checks;
  vendored changes follow CONTRIBUTING.md's replay/history checks.
- Hook changes run `python3 scripts/test-claude-hooks.py`; configuration-validator
  changes run `python3 scripts/test-claude-config.py` as well.

Commit, push and PR creation still require the user's request.

## Scopes

A commit scope is the crate directory name under `crates/` (`agent-runtime`,
`connections-model`, …). Special scopes: `app` (root `src/`), `xtask`, `deps`.

## Commands

```sh
# Choose resource settings locally; cargo-serial.sh is optional.
cargo run
cargo fmt --all --check
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo clippy --workspace --all-targets --features nocterm/user-plugins -- -D warnings
cargo test --workspace --features nocterm/user-plugins
cargo test --locked --manifest-path examples/plugins/focus-timer/Cargo.toml
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path examples/plugins/focus-timer/Cargo.toml
NOCTERM_GUEST_WASM=$PWD/examples/plugins/focus-timer/target/wasm32-unknown-unknown/release/focus_timer.wasm NOCTERM_REQUIRE_WASM_GUEST=1 cargo test -p nocterm --features nocterm/user-plugins user_plugin
NOCTERM_REQUIRE_SSHD=1 cargo test -p nocterm-ssh --test sshd
NOCTERM_REQUIRE_HUB=1 cargo test -p nocterm-hub-io --test hub
NOCTERM_REQUIRE_HUB=1 cargo test -p nocterm hub_live
cargo xtask docs
cargo xtask docs --check
cargo xtask architecture
cargo xtask survey <package> [--json] [--type <Type>]
.claude/hooks/cargo-serial.sh cargo <command>
python3 scripts/test-claude-hooks.py
python3 scripts/check-claude-config.py
python3 scripts/check-plugin-api-compat.py
cargo machete --skip-target-dir
cargo doc --workspace --no-deps -j1
python3 scripts/generate-license-notices.py --check-inputs
```

Linux system dependencies and setup are in [README.md](README.md). GUI checks
require an X11 or Wayland session and Vulkan driver. Regenerate references after
changing schemas, tokens, actions, keymap or internal Cargo dependencies.
Regenerate `THIRD_PARTY_NOTICES.txt` (`python3 scripts/generate-license-notices.py`,
needs `cargo-about` 0.9.2) after changing dependencies, vendored manifests or license
files, or bundled assets; CI and the pre-push hook reject stale notices
([docs/PACKAGING.md](docs/PACKAGING.md)).
