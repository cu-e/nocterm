# Writing a user plugin (preview)

Preview. User plugins are compiled out of release builds. They exist only in a build made with
`cargo build --features user-plugins`, and the contract may still change before API 1.0. The
[reference](reference.md) lists every name and limit this guide uses; it is generated from the
code and always current.

The usual user plugin is a single TOML file. It describes what to draw, which readings of the
application it needs and which settings the user can change. It contains no code: the host reads
the file, checks it, and draws the view itself. A plugin cannot read files, reach the network,
run commands or see anything it did not ask for and the user did not allow.

A plugin that needs logic can instead be a code package: one WebAssembly module that carries the
same manifest in its custom section `nocterm.manifest` and says `runtime = "wasm"`. The host runs
the module in a sandbox with no imports, a fuel budget per call and 16 MiB of memory, and it
answers with a view or effects that go through the same checks and consent as a declarative
package. The reference lists the module's exports and limits ("Code package ABI").
Everything below about the manifest, capabilities and settings applies to both kinds.

## A first plugin

```toml
api = "0.1"
publisher = "example"
id = "clock"
name = "Example Clock"
description = "Shows the local time in the status bar."
capabilities = ["clock.now"]

[reasons]
"clock.now" = "To show the time"

[[contribution]]
local = "face"
kind = "widget"
title = "Time"
icon = "puzzle"
zone = "right"

[contribution.view]
refresh = "next-minute"
nodes = [
  { kind = "row" },
  { kind = "icon", parent = 0, icon = "puzzle" },
  { kind = "text", parent = 0, text = "{settings.label} {clock.now|hh-mm}" },
]

[settings]
version = 1

[[settings.group]]
title = "Clock"

[[settings.group.field]]
key = "label"
label = "Label"
kind = "text"
default = "Time"
max_len = 12
```

Two complete examples are in [`examples/plugins`](../../examples/plugins): a clock widget and a
panel that reads the load of the computer and the running file transfers.

### Identity

The host builds the plugin id from the file: `user-<publisher>-<id>`, so the example above is
`user-example-clock`. The publisher and the id are lower-case letters and digits. Contributions
are named `<plugin id>.<local>`. The publisher is whatever the author wrote; the application
cannot verify it and always shows it as "publisher (unverified)".

A name may not imitate the application: a name or title equal to one of the application's own
panels, plugins or groups is refused.

### Contributions

Each `[[contribution]]` is a `widget` for the status bar or a `panel` for the sidebar. The `zone`
and `order` only suggest where the user may put it. A new plugin never places itself: its items
wait in the tray of Settings > Appearance > Status bar and panels until the user adds them.
Panels also appear under Window > Panels > User plugins, labelled
`Plugin · <name> — <title>`.

Everything a plugin draws is shown in a frame the host owns, with a plugin marker. A panel also
shows the plugin name in a host-drawn label. A plugin cannot hide the frame or draw outside it,
and a widget never grows beyond a fixed width.

### Views

A view is a flat list of nodes. A node names its container with `parent`, the index of an
earlier node. The kinds are `row`, `column`, `text`, `icon`, `meter`, `badge`, `button`,
`toggle`, `list` and `divider`/`spacer`. A `button` or `toggle` has a `key`. The tree is at
most eight deep.

`refresh` says when the view is drawn again: `"next-minute"`, `"next-second"` or
`{ after-ms = 2000 }`. The host never draws more often than every 250 ms, only draws what is on
screen, and draws a plugin that falls behind at a lower rate. A view that cannot be drawn is
replaced by a host-drawn fault chip.

### Templates

Text is literal characters with `{placeholder}` holes. A placeholder reads a capability field,
a setting or the current list item, and may apply filters:

```text
{clock.now|hh-mm}
{monitor.usage.cpu_percent|percent}
{settings.label}
{item.name}
```

The filters are `round(n)`, `percent`, `hh-mm` and `threshold(low, high)`. `threshold` turns a
number into a tone and is used with `tone_when`. There are no functions, no loops and no
concatenation beyond the holes. A value is inserted as plain text; it is never read again as a
template. Write `{{` and `}}` for braces. Every placeholder is checked when the package is
read, so a typo is reported before the user is asked anything.

### Capabilities

A capability is a reading the plugin asks for. List them in `capabilities`; list those that are
nice to have in `optional_capabilities`. The user sees a sentence written by the host for each
one, followed by the reason you wrote in `[reasons]`, quoted as "Author says (unverified)" and
cut to 160 characters. A placeholder can use a capability only when the package declares it.

The user decides about each optional capability when installing. A capability the user did not
grant reads as unavailable and the view shows a dash where its value would be. There is no
switch to revoke a single capability later: the user removes the plugin, or installs an update
that no longer declares it.

### Settings

The `[settings]` table declares fields the user edits in Settings > Plugins. Field kinds are
`toggle`, `choice`, `multi-choice`, `number` and `text`. A group with `contribution` set is
shown beside that item in the status bar editor. Values are stored by the host with the plugin's
data, never in the application's settings file. They are kept across an update when the user
keeps the plugin's data in the update dialog, and are dropped with the data otherwise. When a
field is renamed, `renamed_from` keeps the user's value.

## Check your plugin

```sh
nocterm plugin check path/to/directory
```

reads every top-level `*.toml` and `*.wasm` file in the directory exactly as the application
would, renders each contribution once with nothing observed (a code package is instantiated and
called in its sandbox for this), and prints the name, id, fingerprint, the
capabilities it needs and its contributions. The exit status is 0 when every file passes, 1
when one is refused and 2 for a usage mistake. Symbolic links are not followed.

## What the user sees

1. Settings > Plugins > User plugins > Install from file...: the user picks the file.
2. The consent dialog shows the name, "publisher (unverified)", the fingerprint of the file, and
   one sentence per capability. The buttons are inert for a moment so that a stray click cannot
   accept, and Deny is the default. Optional capabilities are separate choices.
3. The file is stored and the plugin is installed but not placed in any window. The user chooses
   to switch on "Allowed in windows" and adds its items to the layout.

An update is another file with the same id. The dialog says "Replace <name> (unverified
publisher)", shows both fingerprints, highlights what is new (capabilities and settings), and
offers to keep the plugin's data (off by default). An older API version needs a confirmation. A replaced
plugin is not placed until the user allows it again.

A plugin can be switched off without losing its place in the layout, revoked from windows,
removed, and its data removed on its own. These acts are recorded in the audit log shown in the
User section.

## Limits

A manifest is at most 256 KiB (a code package's module at most 2 MiB), declares at most eight contributions and 16 capabilities, and a
view has at most 256 nodes. The reference lists all limits. A package that breaks one is
refused whole, with the reason shown.
