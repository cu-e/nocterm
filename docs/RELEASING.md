# Releasing

Releases are derived from commit history by [release-please](https://github.com/googleapis/release-please). Nobody bumps versions or writes changelogs by hand.

## Flow

```
PR (conventional title) ──squash──▶ master ──▶ release-please updates the "release PR"
                                                      │ merge when ready to ship
                                                      ▼
                                   tag vX.Y.Z + GitHub Release + CHANGELOG.md
                                                      │
                                                      ▼
                                   publish jobs (release.yml, `release_created`)
```

1. Every merge to `master` runs `.github/workflows/release.yml`.
2. release-please opens or updates a single PR titled `chore(master): release X.Y.Z` containing the version bump and the generated changelog.
3. The release workflow regenerates `THIRD_PARTY_NOTICES.txt` on the release PR's
   branch using the pinned Rust toolchain and cargo-about. It commits the generated
   file when needed, then validates the exact prepared commit through CI and all
   four installer builds. The **Conventional Commits** status records successful CI on that SHA;
   **Release validation** records the combined CI and installer result.
4. Merge the release PR only after **Release validation** succeeds. Merging that PR
   **is** the release: it creates the `vX.Y.Z` tag and the GitHub Release.
5. Artifact build/publish jobs run only when `release_created == 'true'`. They
   build the immutable release tag and attach its installers.

Release cadence is a human decision — merge the release PR whenever `master` holds something worth shipping.

## Versioning

[SemVer 2.0](https://semver.org). The bump is computed from commit types since the last tag:

| Commits contain            | `0.y.z` | `≥ 1.0.0` |
| -------------------------- | ------------- | --------- |
| `BREAKING CHANGE` / `!`    | minor         | major     |
| `feat`                     | minor         | minor     |
| `fix`, `perf`, `revert`    | patch         | patch     |
| anything else only         | no release    | no release |

While on `0.y.z` the public surface is considered unstable. Going to `1.0.0` is an explicit decision, made with a `Release-As: 1.0.0` footer on a commit to `master`.

## Special cases

- **Force a version**: empty commit with a `Release-As: X.Y.Z` footer.
- **Fix a changelog entry**: edit the merged PR's description and add a `BEGIN_COMMIT_OVERRIDE` / `END_COMMIT_OVERRIDE` block; release-please re-reads it.
- **Hotfix an old release**: branch `release/X.Y` from the tag, cherry-pick the `fix:` commit, and add that branch to the workflow triggers. Don't create these branches preemptively.
- **Pre-releases**: set `"prerelease": true` and `"prerelease-type": "rc"` in `release-please-config.json` on the branch that should produce `X.Y.Z-rc.N`.

## Files

| File                              | Purpose                                         |
| --------------------------------- | ----------------------------------------------- |
| `release-please-config.json`      | release strategy and changelog sections         |
| `.release-please-manifest.json`   | last released version (managed by the bot)      |
| `version.txt`                     | current version (managed by the bot)            |
| `Cargo.toml`, `Cargo.lock`        | `nocterm` package version (managed by the bot)  |
| `CHANGELOG.md`                    | generated on first release                      |

`release-type` is `simple` (bumps `version.txt`); `extra-files` also bumps the root `nocterm` package in `Cargo.toml` and `Cargo.lock`, so `CARGO_PKG_VERSION` and the Windows file version match the release. Internal crates stay at `0.0.0`.

The Cargo.lock TOML selector accesses `@.name.value` because release-please's
parser wraps scalar values with source-location metadata. Keep the lockfile
selector regression check when upgrading release-please. License fingerprints
continue to cover complete Cargo files, vendored sources, assets, and packaging
inputs; release version changes require notice regeneration too.

## One-time GitHub settings

- Settings → Actions → General → enable **Allow GitHub Actions to create and approve pull requests**.
- Settings → General → Pull Requests: allow **squash merging only**, default commit message **Pull request title**.
- Branch protection on `master`: require PRs, the `Conventional Commits` check,
  and linear history. Review **Release validation** before merging a release PR;
  do not require this release-only status on every development PR.
- PRs opened or updated with `GITHUB_TOKEN` do not trigger pull-request workflows.
  The release workflow explicitly calls reusable CI and Package workflows for the
  prepared SHA and publishes **Conventional Commits** and **Release validation**
  on that SHA. It needs
  `contents: write`, `pull-requests: write`, and `statuses: write`; no separate PAT
  is needed for preparation and validation. Reusable workflow checks appear on
  the triggering workflow, while commit statuses appear on the release PR.
  The bot PR's **Conventional Commits** status succeeds only after the complete
  reusable CI workflow, including its title and commit checks, succeeds.

## Failed publication

A GitHub Release can exist even if its installer jobs fail. Inspect the Package
jobs and release assets before announcing a release. The historical `v1.1.0`
release has no installers because its lockfile and license notices were stale.
Keep its tag unchanged; release the repaired source through the next validated
release PR instead of attaching installers built from a different source tree.

If only an external service failure interrupted publication, retry the failed
Release workflow jobs. Artifact builds must still use the original release tag;
never move that tag to a newer commit.
