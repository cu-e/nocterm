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
optional, so a record can carry data of its own window without a format break. A hub window
carries `"hub":{"organization":"...","sessions":[{"org":"...","id":"..."}]}`, each part omitted
when empty: the organization it showed and the hub sessions its tabs were attached to, in order,
each with the organization it belongs to, since tabs of several organizations can be open at
once.
Older documents have no `hub` and load as before; a `hub` this version cannot read is treated
as empty. A record this version cannot read, such as an environment of a kind a later
build added, is left out and counted; the others are read. Text that cannot be parsed and a
newer `version` are the typed `WindowDocumentError`; both start one local window. The next
save replaces unreadable text, but a document of a newer version is never written over: that
run saves nothing, so going back to the newer build finds the file as it was.

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

**A hub window remembers its organization and sessions.** `HubWindowMemory` is a plain value in
the domain; `OpenWindows::remembered` applies it only to an open window still showing that hub,
so a late report cannot reach a window that was switched to another server. `nocterm-hubs-ui`
reports it through the `HubMemoryKeeper` port, which the application implements over the window
set. A tab joins the memory when it opens; a tab that starts a new session joins it when the hub
reports the id of the session. Closing a tab forgets its session; quitting forgets nothing, so
the sessions of every tab survive a restart. Another organization chosen in the window is
remembered as well.

**Start opens a window per remembered environment.** `windows::restore_windows` opens each
entry through `open_window`, which reports to the set. An entry that cannot open is logged and
skipped. A server that is positively absent from a `[hubs]` that was read is forgotten. A
server that is only unavailable in this run, because `settings.toml` could not be read, its
`[hubs]` section is invalid or hubs did not start, stays in the document at its place, so
fixing the cause brings its window back. When none opens, or nothing was remembered, one
local window opens.

**A restored hub window reattaches its sessions.** The window prefers the remembered
organization: it is selected when the account is a member, else the default one stays, also when
the wish comes while the first read is under way. Once the account is known, which happens when
the hub is online, the sessions of each organization that remembered sessions belong to are
listed, whether or not it is the one shown, and each remembered session that the hub still lists
as starting or running opens a tab on `HubTerminalTarget::Existing`; the screen comes back
through `Event::Restore`. A session the hub no longer lists, such as one that ended meanwhile
or one of an organization the account left, is dropped from the entry without a notice. While
the hub is offline or the device is signed out, nothing is listed: the entry keeps its
sessions, they are reattached when the hub comes online in this run, and otherwise stay for the
next start. A tab that cannot open keeps its session in the entry for the next start; it is not
tried again in this run. A
window that opens its sessions again keeps their order; tabs of one window open in that order.

## Alternatives

A section of `settings.toml` would put application-written state where the user edits and
reviews text, and its sanitising and page rules do not apply. Saving from an `on_app_quit`
handler is refused by the design rules and `Persisted` already captures at shutdown. Keeping the
list in `nocterm-hubs` would tie local windows to a feature a build may not have. A new domain
crate for the document would only re-export `Environment` and `HubAddress`.

## Consequences

- A local window starts with the default tab; only hub windows restore tabs, because only
  their sessions outlive the application.
- The window position and size are the platform's; they are not part of the document.
- The first window a start opens is saved at once, in the background, so a remembered entry
  that did not open and is not kept, such as a removed server, is gone from the file within the
  same start.
- A crash saves the list as of the last change, because every open, switch and close is saved
  in the background.
- `Booted` keeps its field count: the window set joins the hub installation in `Windowing`
  instead of adding a field.
