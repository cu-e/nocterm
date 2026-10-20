---
name: behavior-tests
description: Write tests that pin observable behaviour — characterization tests before a refactor, coverage tests for new code — through public methods, actions, events and rendering, reusing shared fakes. Use before owner-extract or crate-split, and for every new behaviour.
user-invocable: false
---

# Behaviour tests

Rules: AGENTS.md "Tests". Tests that read or write private fields fail review,
and so do files named after review rounds.

## Modes

- **characterize** (before a refactor): cover every behaviour that passes
  through the code about to move, run the tests on the **unchanged** code and
  see them pass. Record the test names in `40-steps.md`; they must stay
  unedited through the refactor.
- **cover** (new or changed behaviour): one test per rule or outcome, including
  refusals and errors (`typed-errors` variants), written red-green where
  practical.

## Where and how

| Layer | Drive through | Example |
| --- | --- | --- |
| domain | public functions and types; plain `#[test]` | parsing, state machines (ADR 0010) |
| adapter | the port trait against real temp files/processes, or the session fakes | `crates/session/src/testing/` |
| service | the handle's public methods in a `gpui_kit::TestAppContext`; `run_until_parked` | store and runtime handles |
| feature/ui | actions, `window.click("<id>")`, `render_frame`, emitted events | `crates/snippets-ui/src/panel/run_tests.rs` |
| app | `bootstrap` with fakes in `Services` | `src/gui_tests/` |

- Extend the existing test file for the behaviour; name the test as a sentence
  about it (`hidden_bottom_refuses_run_and_show_restores_the_same_terminal`).
- Fakes: use `nocterm-session` `test-support` (`InMemoryFs`, `FakeExec`,
  `ScriptedTransport`, `FakeCredentialStore`, `Gate`) and `nocterm-store`
  `test_support`. Extend a fake rather than writing a local copy; propose a
  shared home when none fits.
- Keep a test under about 60 lines (the survey lists longer ones). Move setup
  into a fixture function in the same test module; when the same helper (for
  example a `click` wrapper) already exists in several crates, propose it for
  `gpui_kit` instead of adding another copy.
- Assert outcomes the user or caller sees, not call sequences: the rendered
  state, the event, the persisted document, the returned error variant.

## Run

Through the shared lock, narrowest first:

```sh
.claude/hooks/cargo-serial.sh cargo test -p <package> <test-name-filter>
```

End with the Result block listing tests added and their pass state on the
unchanged code (characterize) or after the change (cover).
