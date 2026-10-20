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

`master` is the release branch and is only changed through pull requests.
`dev` is the integration branch for work being validated before release.

- Choose and record the PR target before branching: `master` for release-ready
  changes, `dev` for integration work. Branch from that target and name the branch
  `<type>/<short-kebab-description>`: `feat/tab-completion`, `fix/resize-crash`.
- Keep branches short-lived and PRs small — one logical change each.
- Changes integrated into `dev` reach `master` through a release-ready PR from
  `dev`. Ordinary PRs into `master` are **squash-merged**: the title becomes the
  commit on `master`, so it must be a valid commit message. Ordinary integration
  PRs into `dev` may use merge commits. If a PR contains both `vendor/` and
  `crates/` or `src/` changes, keep them in separate commits and **rebase-merge**
  the PR to preserve that separation. This also applies when promoting such
  history from `dev` to `master`: neither squash nor a merge commit may combine
  the changes. Correlated commits may need each other to pass checks; the final
  reviewed PR head must pass all required checks. Titles and non-merge commits
  follow Conventional Commits. CI checks the actual before/head history on pushes
  to `dev` and `master`, including commits created by GitHub; this detects invalid
  landings after they occur. Require `CI passed` in branch protection and choose
  the prescribed merge method before landing.
- No force-pushes to `master`, no merge commits on `master`.
- Review against the recorded PR target, using its merge base with the feature
  branch; reviewing integration work against `master` includes unrelated changes.

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
- **Scope** is optional, lower-case kebab, and names the affected crate directory (`ssh`, `agent-runtime`, `connections-model`). Use `app` for the root `src/`, `deps` for dependency bumps and `xtask` for tooling.
- **Body** explains *why*, not *what*; wrap at ~100 chars. Blank line after the header.
- **Breaking changes**: add `!` after the type/scope *and* a `BREAKING CHANGE: <what breaks and how to migrate>` footer.
- **Footers**: `Refs: #123`, `Closes: #123`, `Co-Authored-By: ...`.
- One logical change per commit. If the subject needs "and", split it.

Pick the type by what the *user of the software* sees, not by which files changed: a bug fix that only edits a test helper is `test`, a refactor that fixes a crash is `fix`.

## Code size

A Rust source file may have at most 800 lines, tests included. `cargo xtask architecture`
(run in CI) enforces it. When a file approaches the limit, split it by responsibility into
child modules (`foo.rs` + `foo/part.rs`); move a large `mod tests` into `foo/tests.rs`.
For production code, an extracted module owns a responsibility and its own type;
moving more methods of the same growing type into another file is not an extraction.
The legacy exception list `xtask/oversized-files.toml` is empty; never add an entry to make
a check pass.

`cargo xtask architecture` warns, without failing, at about four fifths of each structural
limit (640 lines, 12 fields, 4 implementation files). `cargo xtask survey <package>` shows
the files, types and module references to design the split from; `--type <Type>` groups a
struct's fields by the methods that use them together.

A function may have at most 80 lines and a cognitive complexity of 20 (`clippy.toml`).
Functions that were already over carry `#[expect(clippy::too_many_lines, reason = "...")]`;
once one is split, the unfulfilled expectation fails clippy, so remove the attribute.
`#[allow]` for these lints is rejected by `cargo xtask architecture`.
Do not add new complexity expectations; extract smaller functions instead.
Architecture checks reject overrides and conditional bypasses for those lints.
Existing expectations require `reason = "predates the limit"`, and their count
per package may only decrease. The exact inventory in `xtask/baseline.toml` also
constrains large structs, implementation spread, public state, globals, shutdown
hooks, string errors and domain I/O. Lower or remove entries when code improves;
never regenerate or raise the inventory. See [architecture guardrails](docs/architecture/guardrails.md).

## Pull requests

Vendored forks have reproducible binary patches against checksum-pinned published
archives. Run `python3 scripts/test-vendor-patches.py` and
`python3 scripts/check-vendor-patches.py` for vendor changes; see
[vendor patch maintenance](vendor/patches/README.md). Keep vendor updates in
separate commits from application changes under `crates/` or `src/`.

- Title: Conventional Commit, as above.
- Description: what, why, how it was verified.
- CI must be green; at least one review (human or the `reviewer` agent plus human sign-off).

## Releases

Fully automated from commit history — see [docs/RELEASING.md](docs/RELEASING.md). Never edit `CHANGELOG.md`, `version.txt` or tags by hand.
