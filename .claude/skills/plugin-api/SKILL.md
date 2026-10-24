---
name: plugin-api
description: Add, change or remove a plugin or anything the window shell hosts. Covers a system plugin (pub static PLUGIN, SystemPlugin with its id, name, description and required flag), a panel or status-bar widget contribution (SystemContribution, register_panel, register_widget, panel and widget factories), a contribution kind, icon, width class or group spec, and the plugin catalog in src/shell_setup.rs. Use when a task asks to expose app data or actions to plugins, user-written widgets or panels, or an API "handle" ("ручка"). No user plugin runtime exists yet, so step 0 stops and escalates instead of adding a getter. Also covers a plugin settings schema (SettingsSchema, FieldGroup, FieldSpec, FieldKind, SettingValue), its SectionBinding to the owner's settings section and the shared plugin settings form. Not for ports between built-in crates (port-design), MCP tools for AI agents (crates/ai tool catalog), GPUI actions or settings sections that no plugin schema exposes.
user-invocable: false
---

# Plugin API: system plugins and contributions

Rules: AGENTS.md "Where code goes" and design rules 2 and 3; crate card
`crates/plugin-model/src/lib.rs`; ADR `docs/adr/0082-system-plugins-and-core-contributions.md`
and `docs/adr/0083-zoned-status-bar.md` (placement, zones, pinned and attention rules) and
`docs/adr/0085-one-plugin-model-with-separate-trust-tiers.md` (settings schema, trust tiers) and
`docs/adr/0086-plugin-enablement-and-the-plugins-page.md` (enablement, the Plugins page, nested pages);
registration types in `crates/workspace/src/workspace/registration.rs`; catalog in
`src/shell_setup.rs`.

## 0. Route
- Consumer is the shell, the layout editor or the Plugins page → this skill.
- Consumer is another built-in feature → `port-design`.
- Consumer is a user plugin or user script → stop. No runtime exists. Never add a
  `Workspace` method, an `Entity` getter or a `pub` field for it. Result `escalate`.

## 1. Declare
A core function (connections, files, the workspace shell) declares
`pub const CORE: &[SystemContribution]` instead; it is never a plugin and never disabled. Do not
turn a core function into a plugin, or a plugin into core, without the owner's decision.
The owner crate's `plugin` module holds `pub static PLUGIN: SystemPlugin` with its id, name,
description, `required` and `contributions`. It is a `static` because an id holds a `Cow`, and a
reference to a `const` of such a value is not promoted to `'static`. The plugin id equals the
contribution namespace. Each contribution is a `SystemContribution` struct literal with id
`<plugin>.<name>`. Panels set `panel: Some(PanelTraits { … })`; widgets set `panel: None`. A new
entry uses `auto: AutoPlace::Tray` unless the task changes Default on purpose. Set
`required: true` only for a plugin that carries attention, permission or notification UI, and
say why.

## 2. Compose
Add a new plugin's `PLUGIN` once to the list in `src/shell_setup.rs`; a core owner's `CORE` goes
to the `core` argument of `plugin::catalog` there.

## 3. Register
In the owner's `register()`, which returns `Result<(), RegistrationError>`:
- panels: `workspace.register_panel(PanelRegistration::new(&DESCRIPTOR, factory))`, adding
  `.expandable()` only for an `ExpandablePanel`. The factory is
  `Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<T>`: read what the panel needs
  from the `&Workspace` argument (sources, active session) instead of capturing it at
  registration. Use an existing workspace source; a missing one is a narrow port designed with
  `port-design`, never a new `Workspace` getter. It may run again after the plugin is
  re-enabled, so keep nothing that must outlive the panel inside it, and never hold the panel entity elsewhere; use the id queries
  (`open_panel`, `toggle_panel`, `panel_is_open`, `panel_is_available`).
  `PanelRegistration::with_entity` is for core contributions and required plugins only; the
  workspace refuses it for a plugin that can be disabled (`RegistrationError::EagerNotRequired`);
- widgets: `workspace.register_widget(WidgetRegistration::new(&DESCRIPTOR, factory))`, where
  `factory` has the same signature and returns `Entity<V>`. The status bar decides when to call
  it; never create the view, or a sampler it needs, eagerly elsewhere.
Actions, services and observers stay in `register()`; they keep running while the plugin is
disabled. Never add `impl Workspace` in a new file.

## 4. Test
Build workspace dependencies with `test_support::deps_for(&[&PLUGIN], cx)`, or `deps_for_core`
when the owner declares `CORE`. Keep the owner's descriptor/panel parity test, the "every catalog
entry is registered" bootstrap test, and the default-geometry oracle unchanged unless the change
intends otherwise.

A widget or panel that shows something native (the local time, a host reading) takes it as a
narrow port through its owner's constructor or `register`, supplied in `src/bootstrap.rs`; do
not add a field to the workspace's `Deps` for a plugin (the workspace's own Clock uses
`Deps::with_clock` because the workspace owns that widget, ADR 0088). A window composed without
the port simply omits the widget. Never reach a native source from a view.

## Settings
1. **Schema.** Add a `FieldGroup` to the owner's `PLUGIN.settings`, with
   `contribution: Some(<id>)` when the field belongs to one widget or panel. Kinds are toggle,
   choice, multi-choice, bounded number and single-line text (`max_len` at most 256,
   sanitized, never masked). There is no colour, secret, password or multi-line field; stop and
   `escalate` (ADR 0085 amendment) instead of adding one or a custom view. Never store a
   credential in a text field.
2. **Storage.** The field key equals the serde key in the owner's own `SettingsSection`, named
   by `settings_section`. Never add a generic table or a second section per plugin.
3. **Binding.** One `SectionBinding::<S>` per plugin, with `read` and `write` functions; writes
   run inside `update_checked`. The plugin's own code keeps its typed `setting::<S>` read. The
   binding sanitizes and bounds text on every read; a text field's owner `sanitize` should call
   `sanitize_text` too, so the owner's own reads are clean.
4. **Tests.** Parity of options and defaults with the owner type; a write through the binding
   changes the expected key; `PLUGIN.settings_section == Some(S::KEY)`.

## Enablement and the plugin's own page (ADR 0086)
1. **Switch.** Never read `[plugins]` or test a plugin's state yourself. The shell creates the
   panels and widgets of a disabled plugin nowhere; ask `Workspace::panel_is_enabled` or the
   resolved layout. A required plugin (`required: true`) cannot be disabled; make a plugin
   required only on the owner's decision.
2. **Own page.** If the plugin keeps a full settings page, set `settings_page: Some(<page id>)`
   in `PLUGIN` and have the application register that page, nested with
   `SettingsPageRegistration::lazy(..).under("plugins", <plugin id>)` or, for a page that is a
   destination of its own (AI), at the top level; a test must keep the two consistent. A plugin without a page sets `None`. The page stays reachable when the plugin
   is disabled.
3. **Anything that runs on its own** (a sampler, a watcher) must stop with the widget or panel
   that drives it; test it with a fake executor while the plugin is disabled.
4. **Plugins page code** (`nocterm-plugins-ui`) names no plugin and takes no settings handle;
   it receives `PluginToggles` and a `PluginSettingsDirectory`.

## 5. Regenerate
`docs/reference/plugins.md` is generated from the catalog. After a plugin, descriptor or
settings change run `.claude/hooks/cargo-serial.sh cargo test -p nocterm plugins_reference`
with `NOCTERM_BLESS=1` set, and commit the result.

## Output (in `30-design.md`)
### Plugin <id>
- Owner crate, name, required (and why), contributions
### Contribution <id>
- Kind, zone/order/group/auto, pinned, panel traits, preview
- Default changes: none | <what and why>
- Tests: <names>

End with the Result block.
