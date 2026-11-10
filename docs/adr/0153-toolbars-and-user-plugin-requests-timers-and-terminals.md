# 0153. Toolbars along the tab area, and requests, timers and terminals for user plugins

Status: accepted. Extends [0082](0082-system-plugins-and-core-contributions.md) with a third
contribution kind, [0089](0089-user-plugin-contract.md) and
[0090](0090-user-plugin-host-and-passive-providers.md) with capabilities that act on their own
and reach the network, and supersedes the part of
[0093](0093-user-plugins-start-off-and-the-api-stays-compatible.md) that shows a held run's User
section without the install button.

## Context

A plugin could draw in two places: a status bar widget and a sidebar panel. Controls that belong
to what the window shows — the controls of a remote service, the state of a build —
had nowhere to sit beside the tabs. A user plugin could only read what the host observed and act
after a click: it could not reach a service of the user's own, keep a credential for one, wake
itself up to refresh, or show a stream of text that the user or an AI chat types into. The host
also never told a user plugin that its contribution was shown, so nothing could start or stop
work with it. On the Plugins page, installing needed a file dialog the page did not offer, and a
run that held user plugins showed no way to install at all.

## Decision

1. **Toolbars.** `ContributionKind::Toolbar` is a strip along one `Edge` of the tab area: top,
   bottom, left or right. A toolbar names its edge; edge and order are unique within a catalog.
   The layout keeps a `Toolbars` table per preset (each edge an ordered list of ids, written only
   when not empty), and `EditOp::ShowToolbar` places a toolbar on an edge or takes it off. A
   toolbar never places itself: it starts in the tray, and the status bar menu (Toolbars) shows
   it on an edge. The top edge is drawn under the tab bar of the window's only tab group, and
   above the tabs when the centre is split. A toolbar's view is created only while it is shown,
   like a widget's. User plugins contribute toolbars with `kind = "toolbar"` and `edge`.
2. **Requests to an address the user typed.** `net.request` sends a request to the origin held
   in a text setting the manifest's `[network]` table names (`origin_setting`). A path below the
   origin, a method and a JSON body are all a plugin chooses: it cannot name a host, the client
   follows no redirect, keeps no cookies and refuses an address that left the origin. The answer
   arrives later as a `response` event under the request's key, cut to 256 KiB; a failure is a
   status-0 response with words, not a fault. At most four requests of a plugin wait at once.
3. **A secret the plugin never reads.** A text setting with `kind = "secret"` is masked in the
   form, left out of every snapshot and template, and allowed only as the `[network]` table's
   `credential_setting`: the host puts it into one header (`Authorization` with a `Bearer `
   prefix by default, or an `X-` header) of requests to that origin. Changing the origin without
   entering a new secret forgets the old one, so a key never follows a changed address.
4. **Timers.** `timers.set` wakes the plugin with a `timer` event after one second to one hour;
   a key that waits is replaced, and a plugin holds at most eight.
5. **Terminals.** `terminals.open` opens a terminal tab of the window after a click, and
   `terminals.write` writes text with colours to it; other control sequences are removed. The tab
   is an ordinary terminal session with a line editor: a line typed into it, by the user or by
   an AI chat it is attached to, arrives as a `terminal-line` event, and closing it sends
   `terminal-closed`. So a plugin can stand a remote console in for a terminal, and the chat's
   attach menu attaches it like any other.
6. **Code views and visibility.** A `code` node shows monospaced text of up to 16 KiB. The
   application tells a plugin's panel and toolbar when they are shown and hidden
   (`visibility-changed`), so timers and requests run only while something shows them.
7. **Installing from the Plugins page.** The User section always shows **Install plugin…** and
   **Open plugins folder**. Plugin files in that folder are listed with an **Install…** button
   that goes through the same consent. The folder is `<config>/user-plugins`, apart from the
   host's own `<state>/plugins`, which is in the same directory on macOS. In a held run the
   folder still opens and a note says to allow user plugins and restart; a build without user
   plugins says how to get one.

Requests and timers are deferred effects the host performs itself, and the providers do not
see them; terminal tabs are opened and written by the window's providers. The host builds a
request from the settings and sends it through the `PluginNet` port (`nocterm-plugin-api`),
whose adapter `nocterm-plugin-net` runs on a background thread and is compiled out with the
feature. A late answer — after the plugin was
replaced, faulted or switched off — is dropped.

## Alternatives rejected

- **Toolbars as status bar zones.** A zone sits at the window's bottom, far from the tabs, and
  its widgets are capped at a fixed width; the toolbar belongs to the tab area.
- **A host allowlist in the manifest.** The author would choose where the plugin goes; the
  address the user types limits it to the service the user means, and nothing else.
- **Giving the plugin its credential.** A plugin that reads a key can send it in a path or a
  body to anything its origin forwards; the host adds it to a header the plugin cannot read.
- **A chat context port that copies a console into the chat.** The chat would read a snapshot
  and could not act; a terminal tab is what the chat already reads from and types into.
- **Network access for declarative plugins.** A request is useful only with code to read the
  answer, and needs a code package's fuel and fault rules.

## Consequences

- The API stays `0.1` while unreleased; the surface and ABI golden files grow with the new
  capabilities, events, effects and node kind.
- A plain `http://` origin is allowed for services on a local network, so a credential entered
  for one travels unencrypted; the settings form shows the address the user typed.
- The layout editor does not yet place toolbars; the status bar menu does.
