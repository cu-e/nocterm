# Agent guide

Instructions for AI agents working in this repository. Humans: see [CONTRIBUTING.md](CONTRIBUTING.md).

## Hard rules

- Follow [CONTRIBUTING.md](CONTRIBUTING.md): Conventional Commits, `<type>/<slug>` branches, never commit to `main` directly.
- Never edit `CHANGELOG.md`, `version.txt`, `.release-please-manifest.json` or create tags — releases are automated ([docs/RELEASING.md](docs/RELEASING.md)).
- Never bypass hooks (`--no-verify`) or weaken a check to make it pass.
- Commit, push and open PRs only when the user asks.
- Report verification honestly: what was run, what passed, what wasn't checked.

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

## Scopes

Commit scopes mirror top-level modules. Add a row when a module appears.

| Scope  | Covers              |
| ------ | ------------------- |
| `deps` | dependency updates  |
| `core` | paths and persistence |
| `design` | design tokens and themes |
| `settings` | settings schema and storage |
| `session` | transport and remote filesystem contracts |
| `ssh` | SSH and SFTP adapter |
| `vt` | terminal emulation and input encoding |
| `ui` | shared UI theme, assets and settings globals |
| `workspace` | window, tabs, panels and actions |
| `terminal` | terminal model and view |
| `connections` | profiles, recents and connection UI |
| `settings-ui` | settings tab |
| `files` | local/remote Explorer and transfer UI |
| `local` | local PTY and shell integration |
| `transfers` | bounded upload/download service |
| `vault` | encrypted credential storage |
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
cargo doc --workspace --no-deps
```

Linux system dependencies and setup are in [README.md](README.md). GUI checks
require an X11 or Wayland session and Vulkan driver. Regenerate references after
changing schemas, tokens, actions, keymap or internal Cargo dependencies.
