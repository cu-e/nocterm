---
name: plugin-api
description: Add, change or remove a plugin or anything the window shell hosts. Covers a system plugin (pub static PLUGIN, SystemPlugin with its id, name, description and required flag), a panel or status-bar widget contribution (SystemContribution, register_panel, register_widget, panel and widget factories), a contribution kind, icon, width class or group spec, and the plugin catalog in src/shell_setup.rs. Use when a task asks to expose app data or actions to plugins, user-written widgets or panels, or an API "handle" ("ручка"). No user plugin runtime exists yet, so step 0 stops and escalates instead of adding a getter. Not for ports between built-in crates (port-design), MCP tools for AI agents (crates/ai tool catalog), GPUI actions or settings sections that no plugin schema exposes.
user-invocable: false
---

# Plugin API: system plugins and contributions

Rules: AGENTS.md "Where code goes" and design rules 2 and 3; crate card
`crates/plugin-model/src/lib.rs`; ADR `docs/adr/0082-system-plugins-and-core-contributions.md`;
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

## Output (in `30-design.md`)
### Plugin <id>
- Owner crate, name, required (and why), contributions
### Contribution <id>
- Kind, zone/order/group/auto, pinned, panel traits, preview
- Default changes: none | <what and why>
- Tests: <names>

End with the Result block.
