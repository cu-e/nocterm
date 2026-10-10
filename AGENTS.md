# Agent guide

Instructions for AI agents working in this repository. Humans: see [CONTRIBUTING.md](CONTRIBUTING.md).

## Hard rules

- Follow [CONTRIBUTING.md](CONTRIBUTING.md): Conventional Commits, `<type>/<slug>` branches, never commit to `master` or `dev` directly. Record the target (`master` release / `dev` integration) and branch from it.
- Never edit `CHANGELOG.md`, `version.txt`, `.release-please-manifest.json` or create tags — releases are automated ([docs/RELEASING.md](docs/RELEASING.md)).
- Never bypass hooks (`--no-verify`) or weaken a check to make it pass.
- Commit, push and open PRs only when the user asks.
- Report verification honestly: what was run, what passed, what wasn't checked.
- Keep Rust files at or under 800 lines ([CONTRIBUTING.md](CONTRIBUTING.md#code-size)): split a
  growing file into child modules by responsibility instead of adding to it. Never raise an
  entry in `xtask/oversized-files.toml`.

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

Layout, settings, snippets and connection state use the shared persistence service.
Runtime chat/favorites and recording remain migration owners. `nocterm-local::fs`
and session `test-support` are not implemented yet; inspect their existing owners
and propose shared extraction instead of describing unavailable APIs.

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
   parent type. Keep vendor changes separate from application code changes.

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

## Pipeline

Non-trivial work goes through four stages. Each stage is a subagent in `.claude/agents/`; the main session orchestrates (run it with `/feature <task>`).

| # | Stage     | Agent         | Writes code | Output                                             |
| - | --------- | ------------- | ----------- | -------------------------------------------------- |
| 1 | Design    | `architect`   | no          | plan: files, steps, risks, test strategy           |
| 2 | Build     | `implementer` | yes         | code + tests on a feature branch                   |
| 3 | Verify    | `tester`      | tests only  | test run results, added coverage, reproduced bugs  |
| 4 | Review    | `reviewer`    | no          | findings ranked by severity, verdict               |

Gates:

- **Design → Build**: the user approves the plan.
- **Verify → Review**: build, lint and tests are green.
- **Review → Done**: no blocking findings. Blocking findings go back to `implementer`; after two failed rounds, stop and escalate to the user.

Trivial changes (typos, one-line fixes, docs) skip the pipeline.
Pass the target branch and verification scope to every stage. Review includes
uncommitted changes as well as commits so it can finish before commit authorization.

## Definition of done

Code changes require `cargo fmt --all --check`,
`cargo build --workspace --all-targets`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`cargo xtask architecture` and `cargo xtask docs --check` to pass. Run additional
checks below when relevant (for example mandatory SSH/SFTP tests for SSH changes).
For documentation-only changes, check links, stale references and generated-doc
freshness; architecture checks still apply. Build, lint and runtime tests may be
omitted because executable inputs did not change.

Report each command and its result. Report omitted or unavailable checks with the
reason. No blocking review findings may remain; commit, push and PR creation
still require the user's request.

## Scopes

Commit scopes mirror top-level modules. Add a row when a module appears.

| Scope  | Covers              |
| ------ | ------------------- |
| `deps` | dependency updates  |
| `core` | paths and persistence |
| `design` | design tokens and themes |
| `themes` | Zed theme import, catalogue and installation |
| `settings` | settings schema and storage |
| `store` | common persistence service and shutdown coordination |
| `session` | transport and remote filesystem contracts |
| `ssh` | SSH and SFTP adapter |
| `vt` | terminal emulation and input encoding |
| `ui` | shared UI theme, assets and settings globals |
| `workspace` | window, tabs, panels and actions |
| `terminal` | terminal model and view |
| `connections` | profiles, recents and connection UI |
| `settings-ui` | settings tab |
| `keymap` | default and user key bindings |
| `keymap-ui` | keymap settings page |
| `ai` | agent registry, thread model, context and tool protocol |
| `acp` | ACP client and local MCP bridge adapter |
| `agent-runtime` | agent connections, admission and chat documents |
| `agent` | agent panel, threads and composer |
| `files` | local/remote Explorer and transfer UI |
| `local` | local PTY and shell integration |
| `transfers` | bounded upload/download service |
| `monitor` | host resource collection, parsing and history |
| `monitor-ui` | status bar monitor, details popup and settings page |
| `snippets` | snippet library, validation and connection bindings |
| `snippets-ui` | contextual snippet panel and highlighted editor |
| `containers` | container engine CLI, listing and actions |
| `containers-ui` | containers panel, logs and shell tabs |
| `vault` | encrypted format and credential rules |
| `vault-store` | vault files, entropy and bounded lifecycle worker |
| `vault-ui` | vault settings and authentication bridge |
| `device-unlock` | native authenticated vault key release |
| `vault-broker` | optional privileged Linux fingerprint broker |
| `app` | application composition and keymap |
| `xtask` | generation and architecture checks |

## Commands

```sh
cargo run
cargo fmt --all --check
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
NOCTERM_REQUIRE_SSHD=1 cargo test -p nocterm-ssh --test sshd
cargo xtask docs
cargo xtask docs --check
cargo xtask architecture
cargo machete --skip-target-dir
cargo doc --workspace --no-deps
python3 scripts/generate-license-notices.py --check-inputs
```

Linux system dependencies and setup are in [README.md](README.md). GUI checks
require an X11 or Wayland session and Vulkan driver. Regenerate references after
changing schemas, tokens, actions, keymap or internal Cargo dependencies.
Regenerate `THIRD_PARTY_NOTICES.txt` (`python3 scripts/generate-license-notices.py`,
needs `cargo-about` 0.9.2) after changing dependencies, vendored manifests or license
files, or bundled assets; CI and the pre-push hook reject stale notices
([docs/PACKAGING.md](docs/PACKAGING.md)).
