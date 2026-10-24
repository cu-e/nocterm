# 0087. The status bar and panels are edited in Appearance through the layout rules

Status: accepted

## Context

[ADR 0083](0083-zoned-status-bar.md) and [ADR 0084](0084-layout-presets-are-app-wide-settings.md)
let a layout be written in `settings.toml`, and [ADR 0086](0086-plugin-enablement-and-the-plugins-page.md)
lets a plugin be switched off. Changing where an item sits still meant editing the file by
hand. The Appearance page is the one place for how the window looks, and its owner is a
different crate from the one that knows layouts and plugins.

## Decision

**Pure edits.** `nocterm-window-layout` owns the rules of an edit: `edit(settings,
legacy_swapped, catalog, target, op)` returns the new settings and the preset the editor
should show, or a typed `EditError`. An operation moves an entry or a group member, moves
it to a zone or group, inserts or removes a separator, takes an item out to the tray or
puts it back, sets the panels that open with the layout and the side of a group, renames,
duplicates, deletes or resets a preset, or restores the stored presets (undo and redo).
Editing a built-in layout in use saves a copy named Custom and selects it, so the built-ins
stay unchanged; a pinned widget never leaves the bar; ids the catalog does not know can
only be forgotten, and the first edit of a stored preset moves the ones it places in the bar
to the tray, where they show as "Not installed" until forgotten. Rules for the result (no leading, trailing or double separators, one
primary group) stay in `resolve`, so an edit cannot produce a layout `resolve` would reshape.

**One checked write.** Every change goes through `SettingsHandle::edit_checked` in a single
update of `[layouts]`, and the editor shows what the write returned, never a guess. A
`[layouts]` table that does not read refuses every edit with a typed error that names it,
and the document is left as it is. The window follows the write through the observation
of ADR 0084, so editing the layout in use changes the footer at once.

**The editor.** The feature crate `nocterm-layout-ui` supplies one section of Appearance
(`layout_section`, an `EmbeddedPageSpec`): the layout in use and the saved ones, a preview
of the bar, the three zones and the panel groups with their members, the tray, and an
inspector that shows where the selected item comes from and the settings it has (the
form of ADR 0085, the same one as on the Plugins page). The preview draws the bar on one
line (icons only, then scrolling, when it does not fit). Items move by dragging, in the preview or in
the lists, onto another place, a group or the tray; each drop is one `EditOp::MoveTo`,
`RemoveToTray` or `PlaceIn` through the same rules. They also move with the pointer buttons, the
item menu and with keys (`Alt+Left`/`Right` within a list, `Alt+Up`/`Down` between zones, `G`
for the next group, `|` for a separator, `Delete` for the tray, `Ctrl+Z`, `Ctrl+Shift+Z`).
Undo and redo keep the presets the editor wrote and are dropped when anything else changes
the layouts; changes to the settings form of an item are never part of them. The editor
reads the catalog and enablement, and holds no layout rule.

**Entry points.** `OpenLayoutSettings { select }` opens Appearance at the section with the
item or group selected. The status bar has a context menu: Hide (not for a pinned item),
Customize Status Bar…, and Layouts. The Plugins page links a plugin to its first item
in the editor. The application places the section in the ordinary Appearance page only;
the startup catalog has none.

This partially supersedes ADR 0057 in one respect: the Appearance page is composed of its
own settings and sections that other features supply (`SettingsPageSpec::with_sections`),
and a page can be asked to scroll to one (`reveal("layout/<id>")`). The ordering, the
required default and the separate startup catalog stand.

## Consequences

- The footer of a window can be arranged without touching the file; the file and the editor
  agree after every edit, including those of other windows.
- The built-in layouts cannot be damaged; the first edit makes a copy.
- The editor lists what the plugin catalog lists, so a contribution a plugin adds to the
  catalog appears in it with no change here.
- The editor depends on the workspace for the checked write and its typed error, a
  feature to shell dependency that other feature crates already have.
