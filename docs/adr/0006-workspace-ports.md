# 0006. The side panel is a state machine; tabs open through a session factory

## Context

`Workspace` kept six independent flags for the side panel, allowing
meaningless combinations such as maximized while closed, and four setters for
the closures that open sessions, background sessions, local shells and
programs.

## Decision

- `RightPanelState` is `Unavailable | Closed | Open { maximized }`. Attention is
  kept beside it, because the toggle shows it whether or not the panel is open.
- `SessionFactory` is one port with a method per kind; the terminal implements
  it (`nocterm_terminal::Factory`) and `main` installs it once. Methods default
  to opening nothing, and single-kind setters remain as overrides of the
  installed factory for tests.

## Alternatives rejected

- A generic capability/`act_as` API for terminal access, session context and
  session specs: three concepts do not justify it.
- Turning each field group into its own entity: adds re-entrancy and updates.

## Consequences

- Invalid side-panel states cannot be represented.
- The workspace still names no feature crate.
