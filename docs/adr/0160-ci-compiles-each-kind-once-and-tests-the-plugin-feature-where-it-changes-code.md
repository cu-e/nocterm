# 0160. CI compiles each kind of build once and tests the plugin feature where it changes code

Status: accepted.

## Context

A pull request waited about 25 minutes for CI, and most of that was repeated work:

- The Rust cache was saved only after a fully green job. Runs kept failing, so every run
  compiled every dependency from scratch.
- A second runner built the whole workspace again with `nocterm/user-plugins` and ran every
  workspace test again. Only six packages declare the feature.
- `cargo xtask` resolves features for xtask alone, so it rebuilt dependencies the workspace
  build had just compiled.
- The architecture check parsed every source file several times with unoptimised syn. That
  took five minutes on a runner.
- A push to `master` built the installers in `package.yml` and again in `release.yml`.
- Small policy scripts took seven runners and seven checks.
- `cargo doc` ran last, so a broken doc link failed CI only after the full test suite.

## Decision

- **Lint and docs** checks the workspace with `cargo clippy`, with and without
  `nocterm/user-plugins`, and runs `cargo doc`. These reuse one set of check artifacts.
- **Build and test** builds the workspace once with `--all-targets`. The tests and the xtask
  binary (`target/debug/xtask docs --check`, `target/debug/xtask architecture`) reuse that
  build.
- **User plugins** builds the example guest. It then tests, with the feature on, only the
  packages that declare `user-plugins` or `user`. The packages come from `cargo metadata`, so a
  new feature owner joins on its own. Every plugin crate is already tested by the workspace
  tests.
- **Vendored crates** replays the vendor archives and runs the vendored tests.
- **Repository policy** runs the hook, configuration, release-pipeline, plugin API, notice
  input, unused-dependency and advisory checks in one runner. Each check runs even after an
  earlier one fails.
- Caches are saved by pushes to `dev` and `master`, also after a failure, and pull requests
  restore them. CI builds carry no debug info. The dev profile optimises `syn`, `proc-macro2`
  and `xtask`.
- `package.yml` runs only from `release.yml` or by hand. The installers no longer wait for the
  notice check. Packaging tools are installed as binaries, not compiled.
- `deny.toml` names each advisory that has no fixed release, with its reason. Every other
  advisory fails CI until the crate is updated.
- `CI passed` waits for every job. `scripts/test-release-pipeline.cjs` fails when a job is
  missing from it.

## Consequences

- With a warm cache the slowest job is the build and test runner, not the sum of every job.
- Locally, the first build after this change recompiles everything that depends on `syn`.
- A test outside the packages that declare the feature does not run with
  `nocterm/user-plugins` in CI. The local gate still runs the whole workspace with the feature
  when plugin code changes.
- A newly published advisory still fails CI on any pull request until the crate is updated or
  the advisory is listed with a reason.
