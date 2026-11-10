# 0130. Cloud agents are offered only in the window of their hub

Status: accepted. Supersedes the window audience of
[0120](0120-cloud-agent-catalog-in-the-chat-menu.md) ("the local window lists every signed-in
hub").

## Context

A hub window is the environment of one hub: it shows the connections and the vault of the
account on that hub, and the window of this computer shows none of them
([HUBS.md](../HUBS.md#the-window-of-a-hub)). ADR 0120 made one exception: the **Cloud** tab of
the new-chat menu in the window of this computer listed the agents of every signed-in hub. A
Cloud agent, its seats and its chats belong to the hub account, so the window of this computer
mixed hub data into the local environment, and its menu needed per-hub headings, colours and
loops that a hub window never uses.

## Decision

`nocterm_agent::CloudAgentsAccess` is installation-wide and offers nothing until
`for_hub_window` binds it to the hub of a window. Only a bound access shows the **Cloud** tab,
and it lists, starts and opens chats of that one hub. The window of this computer has no
**Cloud** tab. `nocterm_cloud_agents_model::arrange` groups the agents of one hub; the
`Audience` type is removed.

Managing agents (the **Cloud agents** page of a server in the Servers settings) is unchanged:
it manages a hub, it does not present the hub's environment.

## Consequences

- A person who wants a Cloud chat opens the window of the hub.
- The Cloud menu has one hub: no hub suffix in section headings and no per-hub iteration.
- The service still tracks every signed-in hub, so switching a window to a hub shows its agents
  at once.
