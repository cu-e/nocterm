# Writing a user plugin (preview)

Preview. User plugins exist only in a build made with `cargo build --features user-plugins`;
release builds are made without it. Until the first release that contains
`crates/plugin-api/api-surface.json` the contract may still change; from that release on a change
that breaks a published plugin needs a new API
version ([compatibility](README.md#compatibility)). The [reference](reference.md) lists every
name and limit this guide uses; it is generated from the code and always current. Editors can
check a manifest against [`manifest.schema.json`](manifest.schema.json).

A build with the feature still starts without user plugins. The user allows them in Settings >
Plugins > User plugins ("Allow user plugins") or with `user_plugins = true` in the `[plugins]`
section of `settings.toml`, and the choice applies at the next start. `nocterm --safe-mode`
starts without any user plugin, whatever the switch says.

The usual user plugin is a single TOML file. It describes what to draw, which readings of the
application it needs and which settings the user can change. It contains no code: the host reads
the file, checks it, and draws the view itself. A plugin cannot read files, reach the network,
run commands or see anything it did not ask for and the user did not allow.

A plugin that needs logic can instead be a code package: one WebAssembly module that carries the
same manifest in its custom section `nocterm.manifest` and says `runtime = "wasm"`. The host runs
the module in a sandbox with no imports, a fuel budget per call and 16 MiB of memory, and it
answers with a view or effects that go through the same checks and consent as a declarative
package. The reference lists the module's exports and limits ("Code package ABI"); [Code plugins](#code-plugins)
below shows how to write one in Rust. Everything below about the manifest, capabilities and settings
applies to both kinds.

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

Each `[[contribution]]` is a `widget` for the status bar, a `panel` for the sidebar or a
`toolbar`, a strip along one `edge` (`top`, `bottom`, `left` or `right`). A toolbar keeps to the
plugin's own terminal tabs and is drawn inside each of them (`tabs = "own"`, the default; a
plugin has one such toolbar); `tabs = "all"` runs it along the window's tab area on every tab,
under the tabs at the top. Across the top or the bottom it is one row, along a side a column.
The `zone`, `edge` and `order` only suggest where the user may put it. Panels and widgets never
place themselves: they wait in the tray of Settings > Appearance > Status bar and panels until
the user adds them, and panels also appear under Window > Panels > User plugins, labelled
`Plugin · <name> — <title>`. A toolbar with `shown = true` starts along its `edge` once the user
turns the plugin on; otherwise it waits too. Every toolbar is listed in the status bar menu under
Toolbars, where the user moves it to another edge or hides it; from then on that choice stays,
whatever the plugin suggests.

Everything a plugin draws is shown in a frame the host owns, with a plugin marker. A panel also
shows the plugin name in a host-drawn label. A plugin cannot hide the frame or draw outside it,
and a widget never grows beyond a fixed width.

### Views

A view is a flat list of nodes. A node names its container with `parent`, the index of an
earlier node. The tree is at most eight deep.

Build panels from the **components** the application's own panels are made of; the host draws
them with the same measures and colours, so a plugin needs no styling to fit in:

| Kind | What it is | Fields (`text`, `detail`, …) |
| --- | --- | --- |
| `section` | A heading in small capitals with an optional count, over its children. With a `key` the user can fold it; the host keeps the fold. | `text` title, `detail` count, `folded` how it starts |
| `item` | A list row: an icon in the node's tone, a title and a muted detail line, then its children (icon buttons, badges) at the end. With a `key` the whole row reports `activated`. | `text` title, `detail` line, `icon` |
| `icon-button` | A small borderless button with an icon; its tooltip names the action. Needs a `key`. | `icon`, `text` label |
| `caption` | A line of small muted text: a hint, an empty list, a state. | `text` |
| `field` | A label and its value on one line; the tone colours the value. | `text` label, `detail` value |
| `chart` | Recent values as a small area chart, as the application draws its own graphs: one or two series of up to 120 values, a line over a light fill, the newest at the right. Full width in a panel, a short line in a bar. Code packages only. | built with `Chart` in the SDK |

The layout primitives are `row`, `column`, `list`, `text`, `icon`, `meter`, `badge`,
`button` (compact), `toggle`, `code` (monospaced text of up to 16 KiB, such as a file) and
`divider`/`spacer`. A `button` or `toggle` has a `key`. Icons inside a view are *glyphs*: the
six plugin icons and common action icons (`play`, `stop`, `refresh`, `terminal`, `folder`,
`file`, `server`, `trash`, …; the reference lists them all). A contribution's own icon, shown
outside the plugin's frame, stays one of the six plugin icons.

A panel's provenance is written by the shell beside the panel's title ("Plugin", with the
plugin's name in its tooltip), where the plugin cannot draw.

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
`toggle`, `choice`, `multi-choice`, `number`, `text` and `secret`. A `secret` is a text field the
form masks and the plugin never reads: it is the credential of the `[network]` table (see
[Requests, timers and terminals](#requests-timers-and-terminals)) and allowed nowhere else. A
group with `contribution` set is
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

## Code plugins

A Rust plugin uses the SDK crate `nocterm-plugin-sdk` (in `crates/plugin-sdk`; use it as a path or a
git dependency). The plugin is a type with `Default` that implements `Plugin`: `render` returns a view for
the contribution `cx.local()` and `on_event` answers an event with effects. The host makes one
value per instance and drops it after a fault, so keep durable state in the plugin's storage
(`storage.set`), which comes back in every snapshot, not in the value.

```rust
use nocterm_plugin_sdk::{Cx, Plugin, ViewBuilder, api::View};

#[derive(Default)]
struct Hello;

impl Plugin for Hello {
    fn render(&mut self, _cx: &mut Cx<'_>) -> View {
        let mut view = ViewBuilder::new();
        view.text(None, "hello");
        view.build()
    }
}

nocterm_plugin_sdk::export!(Hello, concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.toml"));
```

- `cx.observe::<Clock>()` and the other markers in `nocterm_plugin_sdk::observe` read a declared
  capability. They tell a capability the manifest does not declare apart from one the user
  declined.
- `cx.setting("key")` reads a setting, `cx.stored("key")` a stored value, `cx.outcomes()` what
  happened to the previous call's effects, `cx.log(..)` adds a line to the application log.
- `cx.terminal()` names the terminal tab whose toolbar the call draws or reports, and is `None`
  for every other call. `cx.is_column()` says whether a toolbar runs along a side, as a column
  (a narrow rail, at most 240 pixels): show short figures under their icons there, and the full
  row across the top or the bottom. Text in a toolbar is small, as in the status bar.
- `ViewBuilder` has a method for every node kind. The components take small constructors:
  `view.section(parent, Section::new("Servers").count(3).foldable("servers", false))` and
  `view.item(list, Item::new("Build").detail("on · node 1").icon(Glyph::Server).tone(Tone::Success).key("open.1"))`,
  then `view.icon_button(row, "stop.1", Glyph::Stop, "Stop")`, `view.caption(..)`,
  `view.field(parent, "CPU", "38%")` and, under it, `view.chart(parent, Chart::new(cpu).max(100)
  .span(60))`; `.and(values)` adds a second series, such as sent beside received. Keep the
  history yourself, oldest first. A button, toggle, icon button or keyed item takes a key that
  comes back in the `Activated` or `Toggled` event.
- The `effect` module builds the effects: `notify`, `open_panel`, `badge`, `store`, `forget`,
  `request_to`, `timer`, `open_terminal` and `write_terminal`. Each needs the matching capability
  in the manifest. A reply with more than eight effects is
  refused as a whole, none of them applies, and the refusal counts as a fault of the plugin.

The crate type is `cdylib`, and the module is built for `wasm32-unknown-unknown`. `export!` puts
`plugin.toml` into the module as the custom section `nocterm.manifest`, so the `.wasm` file is
the whole package, and writes the exports of the ABI. Write `runtime = "wasm"` in that manifest.
The expansion uses `unsafe`, so the crate must not `forbid(unsafe_code)`.

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
nocterm plugin check target/wasm32-unknown-unknown/release   # a directory holding the .wasm
```

Test the plugin natively with `testing::Host` (enable the SDK feature `testing` for
dev-dependencies). It builds the same snapshots, runs the same encoding and applies the host's
rules to the reply, so a view the host would refuse and an effect without a capability fail in
your test as they would in the application. Set observations, settings and stored values, render
or send events, and read back the stored data and the log.

To try the plugin in the application, with a build that has user plugins (the switch does not
need to be on, since the session loads only this file; `--safe-mode` cannot be combined with it):

```sh
nocterm plugin dev target/wasm32-unknown-unknown/release/my_plugin.wasm
```

The session shows the same consent dialog, labelled DEV, and a notice in each window; the bar item
and the panels of the plugin carry the DEV mark too. The plugin is installed for this session
only: nothing is written to the plugin records, data or package directories, installed plugins
are not loaded, and the plugin is gone when the application closes. The session also reads
`settings.toml` but keeps its changes in memory, so editing the layout or a setting there does
not reach the file. Everything else saves as usual, including connections, snippets, the vault,
saved workspaces, installed themes, layout sizes, the keymap, recordings and agent chats. Edit, rebuild and start the session again, or use
the update flow of the Plugins page.

[`examples/plugins/focus-timer`](../../examples/plugins/focus-timer) is a complete code plugin:
a bar item and a panel, settings, stored state and tests with `testing::Host`.

### Requests, timers and terminals

Code plugins can reach a service of the user's, wake themselves up and own terminal tabs. Each
is a capability the user agrees to, and the answer comes back later as an event.

- **`net.request`** sends a request to the address the user typed into a text setting. The
  manifest names that setting, and optionally a `secret` setting the host sends as a header:

  ```toml
  [network]
  origin_setting = "url"          # a text setting, such as https://service.example.com
  credential_setting = "api_key"  # a secret setting; optional
  credential_header = "Authorization"  # the default; otherwise an X- header
  credential_prefix = "Bearer "        # the default
  ```

  `effect::request_to(key, Method::Get, "/api/items", None)` asks for a path below that
  address; the body, when there is one, is sent as JSON. The answer arrives as
  `Event::Response { key, status, body }`, with the body cut to 256 KiB. Status 0 means no answer
  came: no address is set, the request timed out, or it was refused, and the body says why. The
  host follows no redirect, keeps no cookies and never sends a request elsewhere. At most four
  requests wait at once. When the user changes the address without entering a new secret, the
  secret is forgotten.
- **`timers.set`**: `effect::timer(key, after_ms)` sends `Event::Timer { key }` after one second
  to one hour. Setting a key that waits replaces it; a plugin holds at most eight. The panel and
  the toolbar receive `Event::VisibilityChanged` when they are shown or hidden: keep timers
  running only while something shows them.
- **`terminals.open`**: after a click, `effect::open_terminal(local, terminal, title)` opens a
  terminal tab of the window, or shows it. A line typed into it arrives at `local` as
  `Event::TerminalLine { terminal, line }`; closing the tab sends `Event::TerminalClosed`. The
  tab is an ordinary terminal, so the user can attach it to an AI chat from the chat's attach
  menu and the agent types into it too. The application tells the terminal when the tab shows
  a clean prompt: when it opens, and once the output after a line has been quiet for two seconds
  while nothing is typed. So the user may let the agent type there without asking each time,
  as at a shell prompt. At most four are open per plugin.
- **`terminals.write`**: `effect::write_terminal(terminal, text)` writes to one of them. Line
  breaks, tabs and colour sequences are kept; other control sequences are removed.
- **`terminals.files`**: a tab opened with `effect::open_terminal_with_files(..)` has files the
  plugin serves. The application's file browser shows them while the tab is active, and an AI
  chat the tab is attached to reads and changes them with its usual file tools. Each request
  arrives as `Event::FileRequest { terminal, request, op }`; `op` lists a directory, stats,
  reads, writes, creates a directory, removes or renames, always with absolute paths under
  `/`. The plugin answers with `effect::answer_file(terminal, request, answer)`: `Listed`,
  `Found`, `Content` (base64), `Done` or `Failed`. A request waits at most 30 seconds; a file
  is at most 160 KiB. To keep a binary file's bytes, ask the server with
  `effect::request_bytes(..)`: the body goes as raw bytes and the answer comes back as base64,
  which the plugin passes on unchanged.
- **The bar of the plugin's tabs** needs no capability: declare a toolbar (own tabs is the
  default) and it is drawn in every terminal tab the plugin opens, along the edge the user's
  layout gives it. `render` is called for the toolbar with `cx.terminal()` naming the tab's
  terminal, and what the user does in it reaches `on_event` with `cx.terminal()` set too, so one
  toolbar keeps a bar per terminal apart. Across the top or the bottom it is one row with a
  short host mark in front: text, badges, meters and buttons side by side, and a `spacer` moves
  what follows it to the end. Along a side they stack, at most 240 pixels wide. The bar goes
  when the tab closes. In tests, `testing::Host::render_bar`, `render_bar_in_column` and
  `bar_event` make the same calls.
- **`terminals.bar`**: `effect::hide_terminal_bar(terminal)` takes the toolbar off one tab and
  `effect::show_terminal_bar(terminal)` puts it back. Where it runs is the user's choice.

## What the user sees

0. The user allows external plugins (Settings > Plugins > External plugins > **Allow external
   plugins**) and restarts. Until then nothing can be installed and nothing installed is
   loaded.
1. Settings > Plugins > External plugins > **Install plugin…**: the user picks the file. **Open
   plugins folder** opens the plugins folder; each plugin file in it is listed with
   **Install…**.
2. The consent dialog shows the name, "publisher (unverified)", the fingerprint of the file, and
   one sentence per capability. The buttons are inert for a moment so that a stray click cannot
   accept, and Deny is the default. Optional capabilities are separate choices.
3. The file is stored and the plugin is installed, off. It is listed on the Plugins page beside
   the built-in plugins, marked External. Its **Enabled** switch turns it on (it runs and may be
   placed in windows), and the user adds its items to the layout.
4. **Remove…** beside the plugin's name asks in a dialog what goes: the plugin (its data stays
   for a reinstall), the plugin and its data, or only its data. Nothing goes until the user
   presses the named button; Enter does nothing and Escape closes the dialog.

An update is another file with the same id. The dialog says "Replace <name> (unverified
publisher)", shows both fingerprints, highlights what is new (capabilities and settings), and
offers to keep the plugin's data (off by default). An older API version needs a confirmation. A replaced
plugin is not placed until the user allows it again.

A plugin can be switched off without losing its place in the layout, removed, and its data
removed on its own. These acts are recorded in the audit log shown with the selected plugin.

## Limits

A manifest is at most 256 KiB (a code package's module at most 2 MiB), declares at most eight contributions and 16 capabilities, and a
view has at most 256 nodes. The reference lists all limits. A package that breaks one is
refused whole, with the reason shown.
