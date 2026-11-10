# 0086. Plugin enablement is app-wide and the Plugins page lists plugins with their own pages nested under it

Status: accepted

## Context

[ADR 0082](0082-system-plugins-and-core-contributions.md) put the built-in panels and
widgets into one checked catalog, and [ADR 0085](0085-one-plugin-model-with-separate-trust-tiers.md)
gave a plugin a settings schema. Nothing yet lets a person switch a plugin off, and the
settings of a plugin have no common place: the status bar metrics of Monitor are only on the
Monitor page, which also holds the sampler and the details view.

[ADR 0057](0057-checked-settings-catalog.md) fixes the settings page list as a flat, ordered
catalog in which every page is a navigation entry. A plugin page such as Monitor or
Workspaces is the plugin's own, so it belongs under the plugins it is a part of, not beside
Appearance and the terminal.

## Decision

**Enablement.** `[plugins].disabled` (the `PluginEnablement` section of
`nocterm-plugin-enablement`) lists the ids of the system plugins switched off. It is an
app-wide setting, like the layouts of [ADR 0084](0084-layout-presets-are-app-wide-settings.md),
so every window follows it. Ids the catalog does not know stay in the document and change
nothing; a required plugin (today only AI Agents) is never disabled, whatever the file says
(`effective_disabled`). Core contributions are not plugins and are never disabled. A
`[plugins]` table that does not read runs on the defaults, so every plugin looks enabled; the
Plugins page then refuses to switch a plugin (`EnablementError::Malformed`, from
`PluginEnablement::read_error`) rather than write the section again over what the user wrote.

**What disabled means.** No panel or widget of the plugin is instantiated in any window. Its
panel view is dropped (closing it first, and moving focus only if it was inside), its widget
factory is not called, and what only they drive stops: a disabled Monitor starts no sampling
program. The plugin's actions, services and settings page keep working. The window layout
keeps the plugin's placement, so enabling it again puts it where it was; the resolved layout
leaves the disabled entries out, collapses the separators around them and lists them.
Enabling a plugin creates its panel closed and does not apply the layout's `open` set.
Opening a disabled panel by id, or by its toggle action, opens nothing and says that the
plugin is off.

**The Plugins page.** The lazy guest page `plugins` (feature crate `nocterm-plugins-ui`)
lists the catalog's plugins in order with a chip for a disabled or required plugin, the
plugin's description and its number of contributions. Selecting a plugin shows its detail:
the Enabled switch, the contributions it adds (icon, title and kind), its settings form
(`PluginSettingsForm`, read and written through the plugin's own source) and a link to the
plugin's own page when it names one (`settings_page`). Switching goes through a port, `PluginToggles`, that the
application implements over `[plugins]`; a refusal is typed (`EnablementError`) and shown on
the page. The page holds no storage and no knowledge of any plugin. The action
`OpenPlugins { plugin }` opens it with a plugin selected.

**Nested pages.** A plugin may name a full settings page (`SystemPlugin::settings_page`), and
the Plugins page links to it for any plugin that does, whatever the plugin. A page that
exists for one plugin (Monitor, Workspaces) is nested; a page that is a destination of its
own stays top-level (the required AI Agents plugin names the AI page). A nested page stays
in the immutable settings catalog, registered `.under(parent, anchor)`: it is
hidden from navigation, the parent's entry is highlighted while it is shown and a back link
leads to the parent with the anchor selected. The catalog refuses an unknown parent and a
parent that is itself nested, and a nested page cannot be the default. `open_page_at` opens
any page and asks it to `reveal` an anchor (`SettingsPage::reveal`, default `false`).
`OpenPluginPage { plugin }` opens the page of a plugin whether or not the plugin is enabled.

**Where the Monitor metrics are.** The status bar metrics (`monitor.status_bar`) are edited
on the Plugins page. The Monitor page keeps what governs the sampler and the details view:
whether monitoring runs, the refresh intervals and the details metrics. The key and the
values did not change.

This partially supersedes ADR 0057: the ordinary catalog lists a page that is not a
navigation entry and pages may be revealed at an anchor. The ordering, the required
default, the eager and lazy registration and the separate startup catalog stand. The
startup catalog has no Plugins page. ADR 0056 is unchanged: pages stay with their owners.

## Consequences

- Turning a plugin off is one write that every window follows; the file and the page agree.
- A plugin's page is reachable from the button on its detail in the Plugins page and from
  actions, and a disabled plugin can still be configured.
- A third party plugin tier adds rows to the same page and its own source, and cannot
  disable a required plugin.
- The Monitor page no longer shows the status bar metrics; people who looked for them there
  find them in Settings, Plugins, Monitor.
