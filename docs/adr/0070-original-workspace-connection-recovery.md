# 0070. Workspace authentication belongs to its inner terminal

Status: accepted

## Context

A saved folder refers to a server and ordered jump route. Its original terminal
may be closed, or may live in the local dock or background collection. Opening a
workspace must not require a separate server tab for authentication.

## Decision

Projects searches ordinary central, local and background terminals. Creation
captures the focused ordinary connection's exact Item and session epoch. A live
matching source supplies the existing `ProjectTerminalOpen` provider.

When no connected source exists, the narrow `ProjectTerminalResolve` port asks the
app for a terminal provider. Resolution performs no connection I/O. Connections
resolves saved profiles and checks their destination and complete ordered route;
unknown or changed profiles return typed errors. Direct destinations retain their
ordered jump targets and use normal automatic authentication. Folder metadata
contains no authentication secrets.

The provider creates an ordinary local or SSH terminal directly on the retained
workspace surface. Only its initial working directory changes. Normal host-key,
password and vault prompts render inside that terminal. There is no additional
General server tab or view-owned parent-connection waiter.

Before opening a terminal, Projects checks the current definition and surface
membership. Existing selection receipts prevent deferred callbacks from changing
focus after the user switches away. New terminals and reconnect retain the
provider's connection settings independently of the original server tab.

## Consequences

Opening a saved workspace reuses the ordinary terminal lifecycle and authentication
without requiring its original server tab. The workspace header and sidebar show
the server identity; its outer tab keeps the workspace name.
