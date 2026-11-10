# 0067. Workspaces are outer tabs with their own terminal tabs

Status: accepted

## Context

Server connections must stay available while several folders have their own
terminal tabs, splits and chat history.

## Decision

A workspace is a saved folder on a local or SSH connection and one outer
`ProjectView` Item. It contains a retained `ProjectSurface` using the existing tab
engine. Sidebars, footer, settings and the agent panel remain window-level owners.

The Workspaces footer button follows Containers. Its sidebar groups definitions
for the current connection first and other definitions below. Creation captures
that connection and records the selected folder. Opening an existing workspace
focuses its original outer Item rather than creating a duplicate.

Each inner terminal uses the ordinary terminal lifecycle and a copy of the
original connection's SessionSpec with its initial directory changed. Authentication,
proxy, shell options and ordered jump hosts use their existing owners. The
provider captures the original source once; subsequent tabs and reconnect do not
require the original server tab to remain open. Local and remote folders keep
normal host permissions.

The selected outer Item determines ContextId and visible chat history. Switching
inner terminals keeps that scope. Hidden sessions continue running. Closing the
last inner terminal leaves the outer workspace available for another terminal.
Leaving a workspace selects an ordinary connection; closing it closes its own
sessions. Removing a saved definition deletes metadata only.

UI receipts bind callbacks to the original window, outer Item and inner surface.
Source-bound completions can update a hidden surface without moving focus. Native
pane membership remains owned by the existing dock.

## Consequences

Workspace behavior reuses ordinary sessions, files and application services.
Folder metadata, tab lifetime and chat storage have separate owners. Additional
workspace sources can be introduced when there is an implemented consumer.
