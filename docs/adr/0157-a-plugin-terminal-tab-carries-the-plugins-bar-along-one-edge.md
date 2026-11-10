# 0157. A plugin's terminal tab carries the plugin's bar along one edge

Status: accepted. Extends [0153](0153-toolbars-and-user-plugin-requests-timers-and-terminals.md)
(plugin terminals and toolbars) and
[0154](0154-plugin-views-use-the-applications-components-and-external-plugins-share-the-plugins-page.md)
(plugin views from the application's components).

## Context

A plugin that stands a remote console in for a terminal ([0153](0153-toolbars-and-user-plugin-requests-timers-and-terminals.md))
had its controls and figures for that console only in its panel or in a toolbar of the tab
area. A toolbar belongs to the window, waits in the tray until the user places it, and shows
the same view whatever tab is active, so the state, load and buttons of the server a tab shows
could not sit with that tab.

## Decision

1. **The effect.** `terminals.bar` (`Bar { terminal, edge }`) shows a bar along the top,
   bottom, left or right of one of the plugin's open tabs, moves it, or takes it away
   (`edge: None`). It needs no gesture (the tab was opened after one) and is limited to ten a
   second. It reaches only a tab the plugin opened; the bar is drawn inside that tab, so the
   plugin places nothing in the window by itself.
2. **Who draws it.** The contribution the tab reports its lines to draws the bar, as a
   toolbar (`Kind::Toolbar`), with the snapshot's new `terminal` naming the terminal; what the
   user does in the bar reaches the same contribution with the same `terminal`. In the SDK,
   `cx.terminal()` reads it, `effect::show_terminal_bar` and `hide_terminal_bar` build the
   effect, and `testing::Host::render_bar` and `bar_event` make the calls.
3. **The ABI stays 1.** The wire adds `Call::RenderTerminalBar` and `Call::TerminalBarEvent`
   after the existing variants, and the snapshot on the wire is unchanged: `Call::render` and
   `Call::event` pick the variant from the snapshot's terminal and `Call::into_parts` gives it
   back. A module built before cannot ask for a bar, so it is never sent one of these calls.
4. **The frame.** A bar across the tab is a `FrameKind::Strip`: the host's marker leads and
   the view fills the width, so a spacer moves what follows to the end. A bar along a side is a
   `FrameKind::Rail`: the marker on top and the view stacked under it, 36 to 240 pixels wide.
   Both draw the components as a status bar item does.
5. **Who owns it.** `nocterm-terminal` knows no plugin: `AttachedSession::with_bar` gives an
   attached tab a `TerminalBar` that whoever opened the tab fills, moves or empties, and the
   tab draws it around its screen. The tab owns it, so the bar and its view go when the tab
   closes; the application keeps only a weak handle. A bar's view refreshes with the window's
   other plugin views while it exists, and its renders count against the plugin's budget.

## Alternatives rejected

- **A window toolbar that follows the active tab.** It would need the user to place it, could
  not show two split tabs' bars at once, and the plugin would need to observe the active tab.
- **A field of `terminals.open`.** Adding a field changes the bytes an older module encodes
  `Open` with; a separate effect keeps the ABI and lets a plugin move or take the bar away.
- **A bar contribution kind in the manifest.** Contributions are the shell's catalog and are
  placed by the layout; a bar is part of a tab the plugin opened and has no place of its own.
- **Pushing a view with the effect.** A pushed view would bypass the render budget, the
  refresh cadence, restyling and the fault chip that every other plugin view goes through.

## Consequences

- The capability table, the surface file, the reference and the ABI golden file grow with the
  effect, the two calls and their samples; the snapshot type has a new field for code that
  builds one by hand.
- A bar keeps refreshing while its tab is open in a background tab, at the cadence of the
  plugin's other views.
