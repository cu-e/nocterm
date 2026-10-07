# Contributing

These rules apply equally to humans and AI agents.

## Contribution license

Unless separately agreed in writing, contributions to Nocterm are submitted under
the [PolyForm Perimeter License 1.0.1](LICENSE). By submitting a contribution, you
confirm that you have the right to license it under these terms.

Contributions to the third-party forks under `vendor/`, including local patches,
are instead submitted under the license declared by the corresponding vendored
package: Apache-2.0 for `alacritty_terminal`, `gpui-base` and `gpui-component`, and MIT for
`tid-rs`. Preserve upstream copyright notices and license files, and mark changed
Apache-2.0 files prominently. This policy applies to new contributions; it does
not change the license of code submitted by others under different terms.

## Setup

```sh
scripts/setup.sh   # enables the commit-msg hook
```

## Branching

Trunk-based. `master` is always releasable and is only changed through pull requests.

- Branch from `master`, name it `<type>/<short-kebab-description>`: `feat/tab-completion`, `fix/resize-crash`.
- Keep branches short-lived and PRs small — one logical change each.
- PRs are **squash-merged**. The PR title becomes the commit on `master`, so it must be a valid commit message.
- No force-pushes to `master`, no merge commits on `master`.

## Commit messages

[Conventional Commits 1.0](https://www.conventionalcommits.org/en/v1.0.0/), enforced by the `commit-msg` hook and CI (`scripts/lint-commit-msg.sh`).

```
<type>(<scope>)!: <subject>

<body>

<footers>
```

| Type       | Use for                                         | Release effect |
| ---------- | ----------------------------------------------- | -------------- |
| `feat`     | user-visible capability                         | minor          |
| `fix`      | user-visible bug fix                            | patch          |
| `perf`     | performance improvement, no behaviour change    | patch          |
| `revert`   | reverting an earlier commit                     | patch          |
| `docs`     | documentation only                              | none           |
| `refactor` | restructuring, no behaviour change              | none           |
| `test`     | tests only                                      | none           |
| `build`    | build system, dependencies                      | none           |
| `ci`       | CI configuration                                | none           |
| `style`    | formatting, no logic change                     | none           |
| `chore`    | anything else that doesn't touch shipped code   | none           |

Rules:

- **Header** ≤ 72 chars. Subject in imperative mood, lower-case first letter, no trailing period: `fix(pty): handle zero-size resize`.
- **Scope** is optional, lower-case kebab, and names the affected module (`pty`, `renderer`, `config`). Use `deps` for dependency bumps. Keep the scope list in [AGENTS.md](AGENTS.md) up to date as modules appear.
- **Body** explains *why*, not *what*; wrap at ~100 chars. Blank line after the header.
- **Breaking changes**: add `!` after the type/scope *and* a `BREAKING CHANGE: <what breaks and how to migrate>` footer.
- **Footers**: `Refs: #123`, `Closes: #123`, `Co-Authored-By: ...`.
- One logical change per commit. If the subject needs "and", split it.

Pick the type by what the *user of the software* sees, not by which files changed: a bug fix that only edits a test helper is `test`, a refactor that fixes a crash is `fix`.

## Code size

A Rust source file may have at most 800 lines, tests included. `cargo xtask architecture`
(run in CI) enforces it. When a file approaches the limit, split it by responsibility into
child modules (`foo.rs` + `foo/part.rs`); move a large `mod tests` into `foo/tests.rs`.
Files that were already longer are listed in `xtask/oversized-files.toml`; they may only
shrink, and their entry is lowered or removed as they do. Never raise an entry to make a
check pass.

## Pull requests

- Title: Conventional Commit, as above.
- Description: what, why, how it was verified.
- CI must be green; at least one review (human or the `reviewer` agent plus human sign-off).

## Releases

Fully automated from commit history — see [docs/RELEASING.md](docs/RELEASING.md). Never edit `CHANGELOG.md`, `version.txt` or tags by hand.
