---
name: gate
description: Run the definition-of-done checks through the shared Cargo lock and record exact results, the debt baseline delta and early warnings in 50-gate.md. Quick mode after each step, full mode before review or commit.
user-invocable: false
---

# Gate

Rules: AGENTS.md "Definition of done" and hard rules. Every Cargo command runs
through `.claude/hooks/cargo-serial.sh`, which sets `CARGO_BUILD_JOBS=2` and
holds one lock shared by every agent on the machine; the project hook rejects
compiling Cargo commands that bypass it.

## Quick mode (after each step)

```sh
W=.claude/hooks/cargo-serial.sh
$W cargo fmt --all --check
$W cargo clippy -p <package> --all-targets -- -D warnings
$W cargo test -p <package>
$W cargo xtask architecture
```

## Full mode (before review, commit or "done")

```sh
W=.claude/hooks/cargo-serial.sh
$W cargo fmt --all --check
$W cargo build --workspace --all-targets
$W cargo clippy --workspace --all-targets -- -D warnings
$W cargo test --workspace
$W cargo xtask architecture
$W cargo xtask docs --check
```

Add when relevant:

| Changed | Also run |
| --- | --- |
| SSH/SFTP code | `NOCTERM_REQUIRE_SSHD=1 $W cargo test -p nocterm-ssh --test sshd` |
| dependencies, vendored manifests, licenses, bundled assets | `$W cargo machete --skip-target-dir`; `python3 scripts/generate-license-notices.py --check-inputs` |
| public docs comments | `$W cargo doc --workspace --no-deps -j1` |
| `.claude/hooks/` | `python3 scripts/test-claude-hooks.py` |
| `.claude/skills/`, `.claude/agents/` | `python3 scripts/check-claude-config.py` |

## Interpreting

- A failure stops the gate; fix it, never weaken a check, never `--no-verify`.
- **Baseline**: `git diff xtask/baseline.toml` must show only decreases. If the
  architecture check reports debt that shrank, lower the entry in the same
  change.
- **Early warnings** from `cargo xtask architecture` are not failures. A
  warning for a file or type this change grew means: record it in
  `50-gate.md`, and if it is within one step of the limit, run
  `responsibility-map` before adding more.

## Output: `50-gate.md`

```
# Gate (<quick|full>) <date>
| Command | Result |
| --- | --- |
| cargo fmt --all --check | pass |
…
Baseline delta: <entries lowered | none>
Early warnings touched by this change: <list | none>
Not run: <command — reason>
```

Report exactly what ran and its result; mark anything skipped and why.
End with the Result block (`blocked` if any command failed).
