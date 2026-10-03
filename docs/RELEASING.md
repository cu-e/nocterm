# Releasing

Releases are derived from commit history by [release-please](https://github.com/googleapis/release-please). Nobody bumps versions or writes changelogs by hand.

## Flow

```
PR (conventional title) ──squash──▶ main ──▶ release-please updates the "release PR"
                                                      │ merge when ready to ship
                                                      ▼
                                   tag vX.Y.Z + GitHub Release + CHANGELOG.md
                                                      │
                                                      ▼
                                   publish jobs (release.yml, `release_created`)
```

1. Every merge to `main` runs `.github/workflows/release.yml`.
2. release-please opens or updates a single PR titled `chore(main): release X.Y.Z` containing the version bump and the generated changelog.
3. Merging that PR **is** the release: it creates the `vX.Y.Z` tag and the GitHub Release.
4. Artifact build/publish jobs run only when `release_created == 'true'`.

Release cadence is a human decision — merge the release PR whenever `main` holds something worth shipping.

## Versioning

[SemVer 2.0](https://semver.org). The bump is computed from commit types since the last tag:

| Commits contain            | `0.y.z` (now) | `≥ 1.0.0` |
| -------------------------- | ------------- | --------- |
| `BREAKING CHANGE` / `!`    | minor         | major     |
| `feat`                     | minor         | minor     |
| `fix`, `perf`, `revert`    | patch         | patch     |
| anything else only         | no release    | no release |

While on `0.y.z` the public surface is considered unstable. Going to `1.0.0` is an explicit decision, made with a `Release-As: 1.0.0` footer on a commit to `main`.

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
| `CHANGELOG.md`                    | generated on first release                      |

`release-type` is `simple` (language-agnostic, bumps `version.txt`). Once the stack has a real manifest (`Cargo.toml`, `package.json`, …), switch `release-type` accordingly so that file is bumped too.

## One-time GitHub settings

- Settings → Actions → General → enable **Allow GitHub Actions to create and approve pull requests**.
- Settings → General → Pull Requests: allow **squash merging only**, default commit message **Pull request title**.
- Branch protection on `main`: require PR, require the `Conventional Commits` check, linear history.
- PRs opened with the default `GITHUB_TOKEN` don't trigger other workflows. If required checks must run on the release PR, give release-please a PAT or GitHub App token via the action's `token` input.
