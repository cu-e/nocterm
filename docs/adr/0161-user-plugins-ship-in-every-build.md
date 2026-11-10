# 0161. User plugins ship in every build

Status: accepted.
- Supersedes the compile-out part of [0093](0093-user-plugins-start-off-and-the-api-stays-compatible.md): the cargo feature `user-plugins` and the default build without plugin crates.
- Amends [0160](0160-ci-compiles-each-kind-once-and-tests-the-plugin-feature-where-it-changes-code.md): no separate user-plugin job.

## Context

The user-plugin track was compiled into the application only with `--features nocterm/user-plugins`. Release builds left it out, so no installed Nocterm could run a plugin.

Keeping the flag cost:
- 57 `cfg` gates in 22 files, plus a placeholder module for the build without plugins;
- a second clippy pass and a second test configuration in CI;
- an xtask rule (compile-out) and a release-pipeline test that kept the feature out of releases.

The owner decided that user plugins ship.

## Decision

- The application and its crates depend on the plugin crates unconditionally. `nocterm`, `nocterm-ui`, `nocterm-workspace`, `nocterm-files` and `nocterm-monitor-ui` have no `user-plugins` feature, and `nocterm-plugins-ui` has no `user` feature.
- The runtime gate stays as 0093 decided. A run loads user plugins only when `[plugins] user_plugins` is on (off by default), and `nocterm --safe-mode` loads none.
- The isolation rules check the application's one build. The compile-out rule is gone. The `[isolation.features]` table keeps the test-only and banned features.
- CI runs the workspace tests once, with the example guest built first, so the real WASM guest test runs in **Build and test**.
- The third-party notices cover the plugin runtime's crates.

## Consequences

- Releases install and run user plugins once the user allows them.
- The binary carries the WASM runtime even when no plugin is installed.
- There is one build to check. The plugin crates' isolation rules (I1–I5, guest-only crates) still keep them closed.
