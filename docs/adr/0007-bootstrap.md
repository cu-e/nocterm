# 0007. One bootstrap for the application and its GUI tests

## Context

`main` called about fifteen `init` functions in an implicit order, and the GUI
test fixtures repeated that order by hand, so tests checked wiring the
application did not run.

## Decision

`src/bootstrap.rs` has `bootstrap(Services, cx) -> Booted`, which installs every
global in dependency order, and `build_workspace`, which plugs the features into
a window. `main` and the fixtures differ only in their `Services` (transport,
persistence, local shells, themes, vault). `nocterm_ui::init` returns a
`UiReady` witness that terminal and agent `init` take, so installing them before
the settings store does not compile.

## Alternatives rejected

- A `TerminalReady` witness for `init_credentials`: the credential store does not
  depend on terminal initialization, so the witness would encode no real order.

## Consequences

- GUI tests now run with the session factory, containers panel and credential
  wiring of the real application.
- Tests that re-initialize a feature obtain the witness with
  `UiReady::installed(cx)`.
