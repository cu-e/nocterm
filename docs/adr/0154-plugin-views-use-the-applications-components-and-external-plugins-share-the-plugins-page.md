# 0154. Plugin views use the application's components, and external plugins share the Plugins page

Status: accepted. Extends [0089](0089-user-plugin-contract.md) with components of a view,
supersedes its host-drawn label inside a panel (the shell now writes the provenance beside the
panel's title), and supersedes the separate User section and its two switches of
[0093](0093-user-plugins-start-off-and-the-api-stays-compatible.md) and
[0153](0153-toolbars-and-user-plugin-requests-timers-and-terminals.md).

## Context

User plugins built with the first view contract looked unlike the application. A plugin had
rows, columns, text and outlined buttons, so a list of servers came out as large buttons and
plain text, next to native panels with their compact rows, muted detail lines and small icon
buttons. A panel also showed a second header line ("Plugin · name") under the shell's title. The
Plugins page showed external plugins in a separate User section with its own list and two
switches ("Enabled" and "Allowed in windows") for what a user sees as one choice.

Two reentrant reads crashed the application with a plugin installed. The host applied effects
inside its own update, and a provider that opened a panel or a terminal read the host. The
window activation observer, which runs inside an update of the workspace, refreshed views that
read the workspace.

## Decision

1. **Components.** A view has, besides its layout primitives, the components of the
   application's panels: `section` (a heading in small capitals with a count, foldable with a
   key), `item` (a list row with an icon in its tone, a title, a muted detail line and its
   children at the end, a control with a key), `icon-button` (small, borderless, with a
   tooltip), `caption` and `field`. `nocterm-plugin-view` draws them with the measures and
   colours of the native panels; a plugin styles nothing. The user folds a section and the view
   keeps the fold without asking the plugin. Buttons, badges and text take the same compact
   measures. Declarative views write the components with the `detail` and `folded` keys; the
   SDK builds them with `Section` and `Item` constructors.
2. **Glyphs.** Icons inside a view are glyphs: the six plugin icons and common action icons,
   each mapped by the application to an icon it ships. A contribution's own icon, shown outside
   the plugin's frame, stays one of the six plugin icons, so a switcher cannot pass for one of
   the application's.
3. **Provenance in the shell.** `Panel::provenance` lets the shell write a chip beside a
   panel's title, where the panel cannot draw; a user plugin's panel says "Plugin", with the
   plugin's name in its tooltip. The panel frame only scrolls the content. A bar item keeps its
   marker.
4. **Effects apply after the host's update.** The broker checks every effect inside the host's
   update and returns them in order; the host applies those a window's provider performs once
   that update has ended, and records every outcome in the order the plugin asked. A provider
   may read the host or open a view that does.
5. **Work that reads the workspace waits for its update to end.** Window activation reaches the
   refresh driver after the workspace's update.
6. **One Plugins list.** The Plugins page lists the built-in plugins and, under **External**,
   the installed user plugins, each marked External. One plugin is selected at a time. The
   selected external plugin's detail (facts that wrap, what it may do one line each, one
   **Enabled** switch, its settings, removal and the audit log) shows beside the list; the
   switch that allows external plugins, install, the plugins folder and the consent sit below
   the list. The section draws each part in place (`UserPart`), so its controls stay its own.
7. **One switch.** On enables the plugin and allows it in windows, each only if it is not
   already; off disables it and keeps its place in the layout. An updated plugin, which is not
   allowed in windows until the user says so, shows off.

## Alternatives rejected

- **Style properties on the primitives.** Sizes, paddings and colours chosen by a plugin would
  drift from the application and let a plugin imitate its chrome; components carry no style.
- **Folds as plugin events.** Every fold would cost a round trip and a declarative plugin could
  not keep one.
- **Rendering the User section inside the page's list as one view.** Its controls act on the
  section; drawing its parts in their places keeps them there without a second state.
- **A marker inside the panel.** It cost a line and could be mistaken for the plugin's own; the
  shell's title area is outside the plugin's reach.

## Consequences

- The surface, the ABI golden samples and the generated reference grow with the components and
  glyphs; the API stays `0.1` while unreleased.
- `nocterm-plugin-view` may stop the propagation of a click (a control inside a row), listed in
  the isolation policy.
- A plugin written for the first contract still draws, with the new measures.
