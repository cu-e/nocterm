# 0094. Panels stand alone or in groups the user makes, and the bar preview is the editor

Status: accepted

## Context

[ADR 0081](0081-panel-groups-own-side-panels.md) gave every side panel to a panel group, and
[ADR 0083](0083-zoned-status-bar.md) put every panel of the bar into one: the group it names,
or else the primary group or the assistant's. A group shows one member at a time, so opening
Containers closed Connections when both were in the sidebar, and the only groups were the
catalog's. Within a side the columns ran from the window edge inward, whatever the bar showed.

[ADR 0087](0087-status-bar-and-panels-editor.md) built the editor around three zone lists, an
inspector with the settings of the selected item and a line of help, with the bar preview on
top. The same layout was shown twice, the lists pushed the bar out of view, and the plugin
settings were drawn in two places, here and on the Plugins page.

## Decision

**A panel can stand alone.** A zone entry of `[layouts]` may be `{ panel = "<id>", side = … }`:
one panel that is not in a group. It resolves to a group of one (`ResolvedGroup::is_solo`, keyed
by `solo_group_id`), so the window shows, hides and resizes it with the rules of every other
group, but it is a column of its own: opening it closes no other panel.

**The user makes groups.** A group entry may carry a `name`. The pure rules of
`nocterm-window-layout` gain `GroupWith` (a panel dropped on another joins its group, or the two
make a new one when the other stands alone), `NewGroup`, `Ungroup` (its panels stand alone in its
place; the primary group stays), `RenameGroup`, and moves that take a panel out of its group into
a zone. A new group gets the next free `group-<n>` id. Any group but the primary one needs a panel
to stay, as before.

**Columns follow the bar.** The window lays out the columns of the shown groups in the order the
bar places them, each on its side, so the first panel of the bar is the leftmost column of its
side. Groups the bar does not place come last. Widths stay remembered per width class.

**The preview is the editor.** `nocterm-layout-ui` draws the bar on one line and edits it there:
chips, group pills, gaps between entries that take a drop, the tray of items not in the bar, and a
menu on every chip, pill and separator. A drop on a gap places the item there (a panel stands
alone), a drop on a panel that stands alone makes a group of the two, and a drop on a member puts
the panel in front of it. The menus move, group, ungroup, rename, hide, add separators, choose the
side a centre group opens on and whether a panel opens with the layout; the keys of ADR 0087
stay. There are no zone lists, no inspector and no help line, and a built-in layout that is not in
use offers "Duplicate to edit" instead of a sentence. Rename and delete confirmation take the line
of tools, so the bar does not move.

**Plugin settings live on the Plugins page.** The editor draws no settings form. A chip's menu
opens the plugin's row on the Plugins page (`OpenPlugins`), and its tooltip names the plugin and,
for a user plugin, what the host knows of it (`UserPluginFacts`).

This supersedes, in ADR 0081, that a side panel always belongs to a fixed group and that the
groups of a side run from the window edge inward; in ADR 0083, that every panel of the bar is in a
group; and in ADR 0087, the zone lists, the inspector and its settings form. The rest of those
decisions stand.

## Consequences

- Connections and Containers can be open at once, in the order the bar shows them.
- Stored layouts without `panel` entries or group names read as before; the schema only grows.
- The editor shows each item once, and the plugin settings have one home.
- A right click inside a group opens the menu of the panel under it; the group's own entries are
  in that menu's group submenu and on the pill around the panels.
