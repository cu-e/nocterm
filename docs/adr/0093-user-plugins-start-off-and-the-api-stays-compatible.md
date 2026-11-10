# 0093. User plugins start off, safe mode beats the switch, and the API surface stays compatible

Status: accepted

## Context

[ADR 0089](0089-user-plugin-contract.md) to [ADR 0092](0092-plugin-sdk-and-developer-sessions.md)
built the contract, the host, the code runtime and the developer tooling behind the cargo feature
`user-plugins`. Four things were still open before a user can depend on them:

- **Loading was implied by the build.** A build with the feature restored every installed plugin
  at the start. A user who never asked for user plugins had them, and one whose installed plugin
  misbehaved had no way to start the application without it.
- **Load paths multiplied.** The installed start, the developer session and the `plugin check`
  command each reached the host or a package reader. A rule that holds one path only is not a
  rule.
- **The contract had no promise.** Authors write against `api-surface.json` and the manifest
  keys, and nothing stopped a change from breaking a published plugin.
- **A release build must stay free of the feature** until an outside security review of the
  runtime is done. That is an owner decision; the code must not make it by accident.

## Decision

**One switch, read once.** `[plugins] user_plugins` (boolean, absent while off) allows a build with
the feature to load user plugins. It is read at the start only; changing it applies at the next
start, and the Settings page says so. `nocterm-plugin-enablement` owns the field and the
Plugins page changes it through `UserPluginsSwitch`, a port named for that need, so
`PluginToggles` is not widened. A `[plugins]` table that does not read means not allowed, and the page says why
without a click.

**Safe mode wins.** `nocterm --safe-mode` starts the application with `UserPluginMode::SafeMode`.
It loads nothing user-supplied whatever the switch says, and the page says why. The flag belongs
to the plain start: `nocterm plugin check` and `nocterm plugin dev` refuse it with exit status 2.
`plugin check` is an explicit author command like `dev`: it runs outside the gate, is not held
by the switch, and instantiates the code of the files the author named.

**One gate.** `UserShell::install` is the only construction of the host, the records file and the
data and package stores. A held run (`SafeMode`, or `Installed` while not allowed) opens no
records file, data or package store, creates no runtime or host, restores no plugin and
publishes a static source of system plugins only, marked `UserAvailability::Held`. Windows
attach nothing, the Plugins page shows
the User section without a list and without the install button, and the status bar draws no
user widget. The developer session is not held by the switch: the developer asked for it on the
command line, it loads nothing installed, and it keeps its plugin in memory (ADR 0092). Safe mode
and a developer session cannot be combined: the command line refuses it.

**Widgets that are not drawn are not polled.** A widget the bar did not draw since its last
refresh (overflowed while the menu is closed, or the bar hidden) does not refresh again. Its
polling resumes when it is drawn, including in the overflow menu, which draws the widget's own
view.

**An API with a promise.**

- The manifest keys of the contract are typed data (`MANIFEST_KEYS`: table, key, kind, required,
  vocabulary), so one source feeds the surface file, the reference and the editor schema.
- `cargo xtask docs` writes `docs/plugins/manifest.schema.json` (JSON Schema 2020-12) beside
  `reference.md`, and `docs --check` also fails a hand-written plugin document that names an
  unknown capability id in a code span or names a deprecated one without saying so.
- `scripts/check-plugin-api-compat.py` compares `api-surface.json` with the one at the highest
  `v*` release tag, in a CI job (`plugin-api-compat`, required through `ci-passed`). It fails on
  a removed capability, table, key, value or effect field; a changed family, target, since, field
  type or key entry (type, vocabulary, a narrower integer range); a gesture, a tighter rate, a
  lower live cap or a new live cap; a new limit, an upper limit lowered or a `min-*` limit
  raised; an optional key made required or a new required key; and an `api_version` that went
  down. Any other change except wording needs a higher `api_version`. Entries the base marked
  deprecated may be removed on a higher major version. A base without the file passes with a
  notice, because the guarantee starts with the first release that contains it; a base that
  cannot be read (no tag list, a shallow clone, a broken file) fails the job. The same job holds
  the ABI golden file of code plugins (`abi-golden.json`) to the release: an export or sample of
  the base that changed or vanished needs a higher `abi_version`. An effect request field is
  listed with its type, range and whether it is required, so a retyped, narrowed or newly
  required field fails like a manifest key; a new field has to be optional for the guest.

**Release stays default-off.** Nothing in `package.yml`, `release.yml`, `package-linux.sh` or
`prepare-release-pr.sh` builds with `user-plugins`, and `test-release-pipeline.cjs` fails if one
does. Enabling the feature in a release is a separate decision that waits for the external
security review. It also needs the notices below.

## Alternatives rejected

- **A new start type for safe mode.** `UserPluginMode` already selects how the application loads
  plugins and `bootstrap::Services` holds it as one field; a variant extends the enum, not the
  struct (which is at its field limit).
- **A setter on `PluginToggles`.** The system plugin switch and the user plugin switch have
  different readers and a different timing (now, and at the next start).
- **An environment variable for safe mode.** A flag on the command line is visible in the
  launcher and the process list, and a stale variable cannot hold the application by accident.
- **Holding the developer session by the switch.** It would force the author to change a
  setting they have no other reason to change, to run one file they named.
- **A semver tool for the surface.** The contract is data, not a Rust API; the rules are about
  what a package may rely on.

## Consequences

- **Notices.** `THIRD_PARTY_NOTICES.txt` is generated for the default feature set, so it omits
  the third-party crates that exist only with `user-plugins`: `wasmi`, `wasmi_core`, `wasmi_ir`, `wasmi_collections`, `wasmparser`, `postcard` and `cobs`. A release with the feature
  must regenerate the notices with it first; [PACKAGING](../PACKAGING.md#license-notices) says so.
- A new load path that loads installed plugins must go through `UserShell::install`; a test pins
  that a held run opens no file. `plugin dev` and `plugin check` are the explicit exceptions.
- A held run knows only the system plugins, but its set is marked `Held`, and a layout edit made
  in it keeps the ids of the user tier that the layout names: they stay in their zone, group or
  tray after the entries the bar shows, so a safe-mode start does not move a placed user widget
  to the tray. Only a run that cannot know a plugin keeps its ids; a plugin removed in a normal run
  is still moved to the tray. The default build is held in the same way.
- The surface file is the contract. A change to it is a decision about versions, and a wording
  change is free.
- A running application never shows a change of the switch before a restart; a held run costs
  nothing but the section.
