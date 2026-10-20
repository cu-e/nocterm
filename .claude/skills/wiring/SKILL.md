---
name: wiring
description: Connect a new or changed service, adapter, setting or feature into the application through `src/bootstrap.rs` — explicit dependencies, retained installations, shared persistence and shutdown — without globals or quit handlers. Use whenever code gains a collaborator, persists data or must survive across windows.
user-invocable: false
---

# Wiring

Rules: AGENTS.md design rule 3 and "Reuse before writing"; ADR 0007 (one
bootstrap), 0013 (persistence and shutdown service), 0043/0050–0053 (retained
installations), 0001/0056/0057 (settings sections and pages).

## 1. Dependencies arrive explicitly

- A collaborator is a constructor argument, a field of an `Installed`/`Deps`
  value, or a port object — never `cx.global::<T>()` of another crate, never a
  new `impl Global`, never `on_app_quit`. The architecture check counts all
  three and rejects growth.
- A feature that is installed once and used by every window returns a cloneable
  `Installed` with private collaborators (see
  `crates/snippets-ui/src/installation.rs`). Bootstrap retains it in `Booted`
  (`src/bootstrap.rs`) and passes it to each workspace; views keep the exact
  installation they were opened with.
- If installation can fail, it returns a typed error and bootstrap decides per
  case — follow the nearest precedent, never invent a fallback:
  - a feature whose persistence cannot register for shutdown (snippets,
    connections) is reported and skipped; no unregistered replacement is
    exposed (ADR 0050, 0043);
  - an invalid settings catalog fails bootstrap with `?`; the application logs
    it and quits (ADR 0057);
  - the agent's registration failure is reported and the agent stays
    installed (`src/bootstrap.rs`).

## 2. Order in `bootstrap`

Insert the new step after everything it depends on and before everything that
uses it. The existing order is: settings → UI and shutdown coordinator → themes
→ layout → terminal dependencies → connections → terminal and vault → snippets
→ agent → files → keymap → monitor → settings pages → `Booted`. GUI tests use
the same `bootstrap` with fakes in `Services`; add the fake there too.

## 3. Persistence: pick, do not write

| Need | Use |
| --- | --- |
| A document saved after changes and captured at quit | `nocterm_store::Persisted` + `SnapshotWriter`, registered with `ShutdownCoordinator` |
| A keyed set of documents | `nocterm_store::KeyedPersisted` + `KeyedWriter` |
| An append stream (recordings, logs) | `nocterm_store::AppendPersisted` |
| Ordered background writes without the store | `nocterm_core::persist::{WriteQueue, WriteGate, ShutdownDeadline}` |
| One private file (secrets, state) | `nocterm_core::fs::{write_private_atomic, create_private, AtomicPublication}` |
| User configuration text | `nocterm_core::persist::{save, save_text, save_preserving}` |

Read the symlink and format semantics in ADR 0015 before using a file helper.
Writing `std::fs::write` in a feature or service is a review blocker.

## 4. Settings

- A setting is a field of a `SettingsSection` (`crates/settings/src/section.rs`)
  defined by the crate that uses it, with `KEY` and `sanitize`.
- Its page lives with its owner and joins the checked catalog in
  `src/settings_pages.rs` (ADR 0056, 0057).
- After changing a section run `cargo xtask docs` to regenerate
  `docs/reference/settings.md`.

## 5. Checks

- [ ] no new global, global read across crates, or quit handler
- [ ] the installation is retained, not recreated per window
- [ ] persistence uses a row of the table above and is registered for shutdown
- [ ] bootstrap order respects dependencies; GUI test services updated
- [ ] references regenerated (`cargo xtask docs`)

End with the Result block.
