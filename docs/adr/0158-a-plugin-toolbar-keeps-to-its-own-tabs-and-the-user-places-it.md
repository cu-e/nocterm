# 0158. A plugin's toolbar keeps to its own tabs, and the plugin only suggests where it starts

Status: accepted. Amends [0153](0153-toolbars-and-user-plugin-requests-timers-and-terminals.md)
(a toolbar never places itself) and
[0157](0157-a-plugin-terminal-tab-carries-the-plugins-bar-along-one-edge.md) (who draws a tab's
bar and where).

## Context

A user plugin's toolbar ran along the window's tab area, so a server plugin's controls stood
over every tab, including the user's own shells, and only after the user found it under
Toolbars in the status bar menu. Along a side or the bottom it was drawn as a status bar item,
cut to 36 pixels with only its marker showing, and in the floating-cards layout its card filled
the whole tab area. The bar a plugin put on one of its tabs (0157) had the opposite problem: the
plugin chose its edge and the user could neither move nor hide it.

## Decision

1. **Which tabs.** A toolbar names `tabs = "own"` (the default) or `tabs = "all"`. An own-tabs
   toolbar is drawn inside each terminal tab its plugin opened and nowhere else; an all-tabs
   toolbar runs along the window's tab area as before. A plugin has at most one own-tabs
   toolbar. Built-in toolbars run along every tab (`ToolbarTabs::All`).
2. **Where it starts.** `shown = true` asks the layout to show a toolbar along its `edge` from
   the start (`AutoPlace::Show`), once the user turned the plugin on. Panels and widgets cannot
   ask: they still wait in the tray.
3. **The user's choice wins.** Every toolbar, own-tabs ones included, keeps its edge in the
   layout and is listed under Toolbars in the status bar menu, which moves or hides it. Once
   the user chose, the layout holds the choice and the plugin's suggestion no longer applies;
   no effect can move a toolbar.
4. **How a tab follows.** The workspace registers an own-tabs toolbar without a view
   (`ToolbarRegistration::own_tabs`), tells its edge (`Workspace::toolbar_edge`) and emits
   `WorkspaceEvent::ToolbarsChanged` when an edge changes. The application draws the toolbar in
   each of the plugin's tabs along that edge, with the snapshot naming the terminal, and moves
   or hides the bars when the event comes. `terminals.bar` now only takes the toolbar off one
   tab or puts it back; the edge it names is not read.
5. **The frame.** A toolbar along the top or the bottom is a `Strip` and along a side a `Rail`,
   in the window and in a tab alike; the window draws its view again when it turns. The toolbars
   of an edge share its length, a strip or rail is marked by a short muted line instead of the
   status bar's ring, and in the floating-cards layout each strip is a card of its own size.

## Alternatives rejected

- **A window toolbar shown only while one of its plugin's tabs is active.** It cannot show two
  split tabs' bars at once and needs the active tab's owner in the workspace.
- **A per-plugin bar setting outside the layout.** It would be a second place for the same
  choice; the layout already stores toolbars per preset and the menu already edits it.
- **Letting the effect's edge win until the user chooses.** The layout cannot tell a suggested
  edge from a chosen one, so two suggestions would compete.

## Consequences

- The manifest gains `shown` and `tabs`; the surface file and the reference list them and the
  `tabs` vocabulary. A package that names them is refused by an older host.
- A toolbar that turned from a row into a column is created again, so it loses view state such
  as folded sections.
