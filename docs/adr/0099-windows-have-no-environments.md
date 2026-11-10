# 0099. Windows have no environments: hub sessions live in the ordinary window

Status: accepted

Supersedes [0097](0097-a-window-belongs-to-one-environment.md) and
[0098](0098-windows-and-their-environments-are-restored-after-a-restart.md). Their texts stay
as the history of the decision; the trust rules and the private credential files of 0097 stay in
force.

## Context

[ADR 0097](0097-a-window-belongs-to-one-environment.md) made a window show either this
computer or one hub, and [ADR 0098](0098-windows-and-their-environments-are-restored-after-a-restart.md)
restored those windows after a restart. The cost was a second composition of the window, an
environment slot in the title bar, a confirmation when a window with tabs switched, a list of
open windows with the sessions each remembered, and a hub window that could not offer
connections, files or the agent.

The reason for the separation was the question of which destination the next command, file or
paste reaches when a hub tab sits beside a local one. A hub tab is an attached terminal
([ADR 0095](0095-attached-sessions-restore-and-detach.md)) and says which server it belongs to,
like any other tab of a remote host. Later work routes connections through a hub and keeps
profiles and keys with it, which a window that excludes connections cannot express.

## Decision

**A window has no environment.** There is one composition of the window. `Workspace` gains no
field for an environment and loses the title-bar slot that held the environment menu. New
Window opens the same window as the first one.

**The sessions of a hub are a plugin of the window.** `nocterm-hubs-ui` declares the system
plugin `hubs` with the panel `hubs.sessions`, which is not required and waits in the tray of
the layout until the user places it or runs `OpenHubSessions`. The panel lists the nodes and
sessions of one hub; with several known servers it shows a button for each, and with none it
offers to sign in. A session opens as an attached terminal tab of the ordinary window, and "+"
starts a new one on a node or on any node. The application provides `HubTerminalOpener`
through `AppHubTerminals`, and `bootstrap::build_workspace` registers the feature when the
hub adapter started.

**Nothing about windows is remembered for hubs.** `nocterm-window-set`, its `windows.json`,
`Environment`, `WindowRecord` and the memory of organization and tabs are removed. A tab of a
hub session is an ordinary tab: it is not reattached after a restart, but the session keeps
running on its node and opens again from the panel. `HubWindow` shows the organization the
account belongs to first, and the user picks another in the panel.

**The rest of 0097 stays.** `[hubs]` holds addresses, trust and labels without secrets, the
credentials are private files per hub, trust is explicit, and a changed certificate is never
accepted.

## Alternatives

Keeping environments and adding connections to the hub window would double every feature
registration. Keeping the window set without environments would remember nothing worth
keeping. Reattaching hub tabs after a restart needs the memory of tabs that this decision
removes; the generic restoration of tabs, if the application gains it, serves every tab.

## Consequences

- One window composition; the hub features register next to the local ones and drop out when
  the adapter is absent.
- The workspace crate, the application menus and the startup path return to a single window
  kind.
- Users lose the restoration of hub windows and of the tabs of hub sessions across restarts,
  and the menu that switched a window to a hub.
- The sign-in dialog ends with "Done"; the sessions are in the panel.
