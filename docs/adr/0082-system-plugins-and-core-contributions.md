# 0082. Built-in shell contributions are core or belong to a system plugin

Status: accepted

## Context

The window shell hosted its panels and status bar widgets through type-specific calls:
each feature handed the workspace a view, and the shell knew nothing about what the
view was, who owned it or whether the user could do without it. Placement, titles and
icons lived in the call sites. A status bar that places widgets by hints, and a user who
turns optional features off, both need one place that names every contribution and says
who owns it.

## Decision

A contribution is a panel or a status bar widget with a stable id, a title, an icon and
placement hints. It is either **core** or owned by a **system plugin**.

- Core contributions are functions that are never disabled: the shell's (notifications,
  the local terminal) and those of connections and files. Each owner lists them as
  `plugin::CORE`; they belong to no plugin page. The shell's panel groups are listed as
  `plugin::GROUPS`.
- A system plugin is a named unit compiled into the application. Its owning crate
  declares one `pub static PLUGIN: SystemPlugin` (a `static`, because an id holds a
  `Cow` that a `const` reference cannot promote to `'static`) with its contributions.
  A required plugin cannot be disabled.
- `PluginCatalog` checks the groups, the core contributions and every plugin together:
  ids are well formed and unique; a plugin's contributions sit in its namespace and a
  core contribution sits in no plugin's; exactly one group is primary; group hints name
  declared groups and only panels have one; panel traits exist exactly on panels; order values are unique within a zone. A failure is a typed
  `CatalogError`, and the application's bootstrap refuses to start on it, as it does for
  an invalid settings catalog (ADR 0057).
- The workspace registers under the catalog's ids through one path. A registration names
  a contribution and supplies a factory that borrows the workspace (it may read the
  workspace, including earlier registrations, but cannot update it); the workspace refuses
  an id the catalog does not list, a repeat, a contribution of the other kind, and an expandable panel whose
  contribution draws no header of its own. A panel that exists already (`with_entity`)
  is accepted only for core and required contributions, because the workspace cannot
  drop a panel it did not create when a plugin is disabled. `Deps` carry the catalog;
  `nocterm_workspace::plugin::catalog` builds it and always lists the shell's own
  contributions and groups, so without one the catalog lists those only.

`nocterm-plugin-model` is a domain crate of its own. It holds plain data with no GPUI
types, so feature crates declare plugins without depending on the window shell, and the
shell reads them without a feature depending on its layout types.

## Alternatives

- A command or service registry: ADR 0002 and ADRs 0057 and 0077 keep dependencies
  explicit; a catalog of descriptors passes no behaviour and looks nothing up at run
  time.
- Put the descriptors in the workspace crate: every feature crate that declares a plugin
  would depend on the window shell for data alone.
- Treat every built-in as a plugin: the shell's own functions would be disableable, or
  would need a required flag with no meaning.

## Consequences

- A feature adds a panel or widget by declaring it in its plugin and registering a
  factory; the application lists the plugin once in `src/shell_setup.rs`.
- Placement, titles and icons are data the shell can read before any view exists.
- The old type-based panel calls remain beside the catalog path until the panels move
  to it.
