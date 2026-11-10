---
name: gate
description: Select local checks by changed inputs, reuse valid evidence, and finish the applicable Definition of done without per-edit or per-stage command bundles.
user-invocable: false
---

# Verification

AGENTS.md sets the workflow and requires this procedure's completion checks.
This file owns the detailed check matrix and commands; roles refer here rather
than copying them. Required CI is independent. Cargo resource settings follow
CONTRIBUTING.md; the coordination helper is optional.

## Select by the uncertainty

| Question/change | Useful scoped check |
| --- | --- |
| Reproduced regression | One `cargo test -p <package> --lib <filter>` or existing integration target/filter |
| Behavior spans an owner/package | Affected test targets and consumers |
| API/compilation uncertainty | `cargo check -p <package> --lib` plus affected consumers, unless tests already compile them |
| Lint failure | Clippy for the affected package/targets/features |
| Rust formatting | Format changed sources; use `cargo fmt --all --check` for completion |
| Rendering, caching or other hot paths | Use `performance` for representative cost and correctness evidence; choose relevant GUI/behavior tests |
| Ownership/guarded structure | Inspect changed counts; survey when useful; run architecture for affected guardrails |
| Skills/agent instructions | Configuration validator, links and stale references |
| Hooks/scripts | Their behavior tests and language syntax/configuration checks |
| Prose only | Links/references; Cargo/generated checks only for affected generator inputs |

Finish a coherent edit before checking. Do not edit checked inputs while their
check runs. A test already compiles its target; do not routinely precede it with
both check and build. In parallel work, coordinate one executor and avoid
queuing the same pending check twice.

## Completion and failures

At integrated completion, choose the applicable AGENTS.md scope after review
fixes. Broad shared changes, uncertain workspace impact or an explicit
full-verification request need the full workspace suite below. A small change
does not inherit that suite merely because a stage or worktree ended. Apply
required plugin-feature,
SSH/hub, guest, dependency/notice and vendor checks when their inputs change;
confirm fixtures actually ran, rather than counting skipped/zero tests.

Rerun a failing target first. Preserve successful evidence when relevant
source/configuration is unchanged; identify which results a fix invalidates.
A new stage/report does not invalidate evidence. Reviewers inspect it and rerun
only missing, stale or specifically suspect checks. Keep stable targets and
investigate concrete stale artifacts before cleaning or touching all sources.
Do not change checks, architecture limits or baselines to obtain a pass.

## Evidence

One compact evidence section in the plan is enough:

```text
Command | Source/configuration and targets/features | Result | Log if needed
Reused: <prior evidence and why it still applies>
Pending/unavailable: <check, reason and scope limitation>
```

Record the worktree/source snapshot, including relevant uncommitted/untracked
inputs. Separate queue wait and execution time when available. Report scoped
coverage as scoped; a completion pass does not assert a full-workspace pass.

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
  `src/shell_setup/user_plugin*`) is part of the default build and its workspace checks.
  Plugin SDK/example changes also build/test the example and run the gated
  WASM guest test. Preserve API/ABI compatibility checks for contract changes.
- Dependencies/manifests require applicable unused-dependency and notice checks;
  vendored changes follow CONTRIBUTING.md's replay/history checks.
- Hook changes run `python3 scripts/test-claude-hooks.py`; configuration-validator
  changes run `python3 scripts/test-claude-config.py` as well.

Commit, push and PR creation still require the user's request.

## Commands

```sh
# Choose resource settings locally; cargo-serial.sh is optional.
cargo run
cargo fmt --all --check
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --locked --manifest-path examples/plugins/focus-timer/Cargo.toml
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path examples/plugins/focus-timer/Cargo.toml
NOCTERM_GUEST_WASM=$PWD/examples/plugins/focus-timer/target/wasm32-unknown-unknown/release/focus_timer.wasm NOCTERM_REQUIRE_WASM_GUEST=1 cargo test -p nocterm user_plugin
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
python3 scripts/test-claude-config.py
python3 scripts/check-plugin-api-compat.py
cargo machete --skip-target-dir
cargo doc --workspace --no-deps -j1
python3 scripts/generate-license-notices.py --check-inputs
```

Linux system dependencies and setup are in [README.md](../../../README.md). GUI checks
require an X11 or Wayland session and Vulkan driver. Regenerate references after
changing schemas, tokens, actions, keymap or internal Cargo dependencies.
Regenerate `THIRD_PARTY_NOTICES.txt` (`python3 scripts/generate-license-notices.py`,
needs `cargo-about` 0.9.2) after changing dependencies, vendored manifests or license
files, or bundled assets; CI and the pre-push hook reject stale notices
([docs/PACKAGING.md](../../../docs/PACKAGING.md)).
