# 0097. A window belongs to one environment

Status: accepted

## Context

Nocterm opens windows on this computer: local shells, SSH connections, files, containers,
workspaces, the monitor and the agent. A Nocterm hub is a server that keeps terminal sessions
alive on its nodes. A user wants to open those sessions from the application, from more than
one computer, without mixing them into the local tabs, connections and sidebars: a hub tab that
sits beside a local tab invites the question of which of the two the next command, file or
paste reaches.

Two things have to be decided: what "being on a hub" means for a window, and how the
application learns to trust a server and keep the right to speak to it.

## Decision

**A window belongs to one environment, `Local` or `Hub(HubId)`, chosen when it is composed.**
`nocterm_hubs_model::Environment` is an input of the composition: `windows::open_window` takes
it with the retained installations, and the window never changes what it is. `Workspace` does
not know about environments and gains no field for one. Registration of panels and factories
happens once per `Workspace`, so a window does not switch features on and off; the
composition builds a different `Workspace` instead.

**A hub window has its own feature set.** `build_hub_workspace` registers the hub's sessions
panel (`hubs.sessions`, a core contribution of `nocterm-hubs-ui`, present only in a hub
window), the "+" menu that creates a session on a node, Settings, Appearance, Keymap, Plugins
and Snippets, and the hub's application menu. It does not register the local terminal,
connections, files, containers, workspaces, the monitor, the agent, the vault prompt or user
plugins. The session factory stays the one that opens nothing, so a hub window cannot start a
local shell. A panel the host does not register is absent from the footer and from panel
cycling by the behaviour the window already has for unavailable panels. Layout and settings
stay app-wide ([ADR 0084](0084-layout-presets-are-app-wide-settings.md),
[ADR 0086](0086-plugin-enablement-and-the-plugins-page.md)).

**Switching a window replaces its root.** For "this window" the application composes a new
`Workspace` for the chosen environment and replaces the root view of the same native window,
then updates the title and focus. Closing the window and opening another would move it where the client cannot place a
window (Wayland), and closing the only window quits the
application. The old `Workspace` is released: local sessions are closed, tabs of a hub are
detached ([ADR 0095](0095-attached-sessions-restore-and-detach.md)). When a local window
holds tabs, the menu asks first, with "Open in a new window" as the default; a hub window
switches without asking because its sessions keep running on their nodes. "New window"
always opens the local environment; a hub is opened from the menu.

**The environment menu lives in the title bar.** The workspace offers a slot
(`set_environment_menu`) that the composition fills with the feature's view. The menu lists
"Local", every known hub with its status, and "Sign in to a server". The status is the one
all windows share: online, connecting, offline with the time of the next attempt, signed
out, revoked, certificate changed, or a protocol version the application cannot speak.
`EnvironmentOpener`, the port by which the menu asks for a window, is declared by the
feature and implemented by the application, since only the application can compose windows;
the same holds for `HubTerminalOpener`, because features do not depend on each other.

**The list of servers is a settings section without secrets.** `[hubs]` in `settings.toml`
holds, for each server, its address, how it is trusted (a pinned fingerprint or the public
certificate authorities) and a label. It is saved through the settings handle in a single
checked edit. If the edit fails, the credentials just written are erased again, so no
server exists in one place and not the other.

**Device keys and tokens live in a private file per hub.** The adapter keeps a device key and
the tokens of one hub in a mode-0600 file under `hubs/` of the state directory, named after the
host and port, published atomically and read under an inter-process lock
([ADR 0015](0015-private-file-publication.md)); the lock is needed because a refresh token
spent twice revokes the device. The file is erased on sign-out, on revocation and when the
server is removed. These secrets never enter `settings.toml`, the vault or the log. The vault
may be locked when a hub is opened and has its own format and lifecycle, and an operating
system keyring would add a native dependency; a stronger store can replace the file behind the
same port later.

**Trust is explicit and a changed certificate is a hard error.** Before any credential is
sent, the adapter reads the certificate the address presents and the model decides:

- an entered fingerprint that matches pins it; one that does not match is an error with only
  "Back" as a way forward;
- no fingerprint and a chain valid for the name under the public authorities trusts those
  authorities and pins nothing, so a certificate that rotates every few weeks does not break
  the server;
- no fingerprint and a self-signed certificate is trusted on first use only when the user
  confirms the fingerprint shown (the server's setup prints the same value); the confirmed
  fingerprint is pinned;
- a pinned server whose certificate differs is `TrustFailed` with both fingerprints. It is
  never retried and offers no "accept": the user removes the server and signs in again.

`http://` addresses are refused. The user may edit the pinned fingerprint in `settings.toml`
by hand, as with `known_hosts`.

## Alternatives

A mixed window, with hubs as one more source of connections and sessions beside local ones,
would route every request through a destination and refactor `ConnectRequest` and
`SessionSpec`; it was rejected because the user asked for separate environments. An
environment as a field of `Workspace` with features switched inside it needs unregistering,
which no panel or factory supports and which would be a risky new mechanism; composing a new
`Workspace` reuses the existing composition. A hub method on `SessionFactory` would widen a
broad trait ([ADR 0006](0006-workspace-ports.md)); the attached tab takes its own transport
instead. Closing a window and opening another loses its position and, with one window, quits.
Storing credentials in the vault or the keyring was rejected above, and trusting a changed
fingerprint after a prompt would make the pin worthless against the attack it exists for.

## Consequences

- The local window is unchanged except for the menu in its title bar; a composition without
  hub dependencies has no menu and every existing behaviour.
- Adding a feature to a hub window is a registration in `build_hub_workspace`, not a new
  state of `Workspace`. A later feature that needs several environments in one window would
  need a new decision.
- Windows do not come back after a restart, except by the user choosing an environment; the
  sign-in does persist because the credentials are on disk.
- Several windows of one hub share one status, one connection and one list of sessions; the
  hub's sessions are the only thing that outlives a window.
- The application keeps one extra installation, the hubs feature, in its retained
  installations, and `Booted` is at its field limit; the next addition groups its fields.
- Sign-in to a server whose settings file cannot be written fails visibly.
