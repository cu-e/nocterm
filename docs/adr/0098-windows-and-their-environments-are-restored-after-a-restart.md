# 0098. Windows and their environments are restored after a restart

Status: accepted

## Context

[ADR 0097](0097-a-window-belongs-to-one-environment.md) gives every window one environment,
`Local` or one hub, and noted that windows did not come back after a restart: the user chose the
server again each time. A user who works in two windows, one on a hub, wants the same two after
quitting and starting again. The sessions of a hub outlive the application, so only the list of
windows is missing.

## Decision

**The environments of the open windows are a state document, not a setting.**
`state_dir/windows.json` is `{"version":1,"windows":[{"environment":{"kind":"local"}}, ...]}`,
in the order the windows opened. It is state the application writes, so the user does not edit
it and it is not in `settings.toml`. Records ignore unknown fields and every later field is
optional, so a record can carry data of its own window, such as the sessions it shows, without
a format break. A newer `version` and text that cannot be parsed are the typed
`WindowDocumentError`; both start one local window and the next save replaces the file.

**The rules are pure and live in the domain.** `nocterm_hubs_model::windows` holds the document
and `OpenWindows<K>`, the live list keyed by any window identity:

- a window that opens is appended; switching "this window" replaces its environment;
- a window the user closes is removed, except the last one: closing the last window quits the
  application, so it stays as the session the next start restores;
- quitting removes nothing.

**A service keeps the list and saves it.** `nocterm-window-set` holds `OpenWindows<WindowId>`
in one `nocterm_store::Persisted` entity registered with the `ShutdownCoordinator`
([ADR 0013](0013-persistence-shutdown-service.md)). It subscribes to window closing once, for
the life of the application, to apply `closed`; it installs no global and registers no quit
handler. GPUI drops the windows after the quit observers ran and reports no close for them,
so the single capture at shutdown writes every window. The service is not part of
`nocterm-hubs`: a build without hubs has windows too, and hubs must not own them.

**The application reads and writes the file.** `src/window_set_setup.rs` reads the document
before the first window opens and supplies the writer (`persist::save_text`); a run without
files remembers nothing. `src/bootstrap/windowing.rs` groups the hub servers and the window
set into one `Windowing` owner of `Booted`, which was at its field limit. If the save cannot be
registered for shutdown, windows work and are not remembered.

**Start opens a window per remembered environment.** `windows::restore_windows` opens each
entry through `open_window`, which reports to the set. An entry that cannot open, such as a
server removed from `[hubs]`, is logged and skipped. When none opens, or nothing was
remembered, one local window opens.

## Alternatives

A section of `settings.toml` would put application-written state where the user edits and
reviews text, and its sanitising and page rules do not apply. Saving from an `on_app_quit`
handler is refused by the design rules and `Persisted` already captures at shutdown. Keeping the
list in `nocterm-hubs` would tie local windows to a feature a build may not have. A new domain
crate for the document would only re-export `Environment` and `HubAddress`.

## Consequences

- Restoring a window opens its environment, not its tabs: a local window starts with the
  default tab and hub tabs are not reattached. A later decision can add per-window data to the
  record.
- The window position and size are the platform's; they are not part of the document.
- A window opened by a failed restore does not erase its entry until a later save, which
  writes only the windows that opened.
- A crash saves the list as of the last change, because every open, switch and close is saved
  in the background.
- `Booted` has one field of room again.
