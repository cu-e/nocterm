# Architecture guardrails

Part of the [architecture overview](../ARCHITECTURE.md). Run
`cargo xtask architecture` after changing source or crate boundaries.

The check combines the Cargo dependency layers, the 800-line file inventory,
complexity-attribute policy, and the AST debt inventory in
[`xtask/baseline.toml`](../../xtask/baseline.toml). The inventory records actual
existing exceptions, not a budget to spend elsewhere. A new exception or growth
fails. A reduction also fails until its entry is lowered or removed. No command
regenerates the inventory.

Agent protocol SDK packages (`agent-client-protocol*`) may be dependencies of
any kind only in `nocterm-acp`; renamed bindings do not escape the check
([ADR 0058](../adr/0058-neutral-agent-protocol-boundary.md)).

Every first-party workspace package must inherit `[lints] workspace = true` in
its actual Cargo manifest. The check uses Cargo's complete no-dependencies
workspace membership, including the root package, binaries and proc macros;
missing or inconsistent package IDs fail closed. Manifest TOML is parsed
semantically, so comments and lookalike metadata do not establish inheritance.
Root, `crates/` and `xtask/` packages are first-party; `vendor/` retains upstream
lint ownership. Unsupported or outside workspace member locations fail rather
than escaping the policy. This protects the workspace lint rules in addition to
the source expectation inventory.

## What is measured

| Metric | Identity and rule |
| --- | --- |
| Struct fields | Production structs with more than 15 fields, including tuple fields |
| Public fields | All public or restricted-visible fields in feature/service structs, except explicitly named plain data values |
| Impl spread | More than four distinct source files implementing a local type; inherent and trait implementations count together |
| Complexity expectations | Guarded lint expectations per package, including tests and examples |
| Globals | Every production implementation of the GPUI Global trait, by the fully qualified implementing type |
| Global reads | Production calls to the panicking `global` method, per package |
| Quit handlers | Production `on_app_quit` calls, by their enclosing function or method |
| String errors | Production Result types with a String error, per package; imports and type aliases resolve |
| Domain dependencies | Non-development dependencies on filesystem, network, process or image adapters |
| Domain I/O references | Resolved filesystem, network, process and prohibited dependency paths, by module and API |

Cargo library and binary targets supply the production module roots. Inline,
external and `#[path]` modules retain their Rust module identities. Same-named
local types in different modules stay distinct; imports, reexports and type
aliases resolve to their declarations. Test-only modules, functions and fields
are excluded from production metrics. Other conditional production branches are
included, even when the current host does not build them. Temporary-file and
directory adapters (`tempfile`), including aliased constructors, count as I/O;
development-only dependencies and their `#[cfg(test)]` uses remain excluded.

Conditional module paths (including nested `cfg_attr`) and paths on inline modules
are rejected with an explicit diagnostic until all source alternatives can be
loaded. Conflicting imports fail closed, including across files declaring the
same module. Conditional type aliases preserve every error and I/O alternative;
implementation-owner aliases that name different types fail with a diagnostic.
Platform-specific reexports of verified local concrete structs retain the owner
selected by matching implementation conditions; unresolved alternatives fail.
Ambiguous types exposed by competing local globs require explicit bindings.
External glob imports
require explicit names; the verified GPUI presentation prelude is supported.
Local globs retain external reexports, so exporting an I/O function through a
local module does not make it pure. Ordinary local and cyclic glob resolution
remains supported.

[`architecture-policy.toml`](../../xtask/architecture-policy.toml) lists individual
plain data values allowed to expose fields, with the reason for each exemption.
It does not exempt entire directories, crates, derives or names ending in
Settings. Adding a type there requires design review; stateful entities are never
plain data exemptions.

Both guarded complexity lints require an existing expectation with the reason
`predates the limit`. `allow`, missing or different reasons, and conditional
attributes that contain them fail. Comments and string literals are parsed as
data, rather than mistaken for attributes.

This is a source check, not Rust compiler expansion or semantic analysis. It does
not expand procedural or declarative macros, prove whether a method performs
I/O, or validate every dependency API. Review still checks generated state,
indirect I/O, responsibility boundaries, and dependencies obtained through
associated accessors. Function- and const-local declarations, including inline
modules, are measured with separate lexical identities. An out-of-line module
nested inside a function or const is rejected with a diagnostic until its path
loading is implemented; it cannot silently escape the inventory. The same
fail-closed policy rejects `include!` source, including literal, dynamic and
locally aliased calls: declare a Rust module instead. Passing the
inventory does not waive the design rules.

## Crate cards

Every package under `crates/` starts its crate root (library, else binary) with
an ownership card in its `//!` documentation: `Owns:`, `Does not own:`, its
`Ports`, `Settings` section, `Globals` and `Invariants`. The label may carry a
short qualifier before its colon (`Ports used:`, `Settings section:`). The
check is a presence check on crate-level documentation; it does not judge
whether the card describes the actual boundary, which remains a review duty.

## Early warnings and the survey

`cargo xtask architecture` also prints, without failing, each package that has
reached about four fifths of a structural limit: files of 640 lines or more,
structs with 12 or more fields and types implemented in four files. A warning
asks for a split by responsibility to be designed before the limit forces one.

`cargo xtask survey <package> [--json] [--type <Type>]` reports the facts such
a design needs: file sizes, struct shapes and visibility, implementation spread,
references from each top-level module to workspace crates and sibling modules,
tests longer than 60 lines and the same warnings. `--type` adds the method ×
field matrix of one struct with candidate owners: fields that the same methods
use together, coordinators that reach most of the state, fields only they use,
cross-cluster methods and methods without direct field access. The survey is syntactic and
keys types by name; it refuses a matrix for an ambiguous name and is a planning
aid, not a check.

## Isolation of the plugin crates

The user plugin crates (`nocterm-plugin-model`, `-api`, `-abi`, `-wasm`, `-packages`, `-ports`,
`-view` and `-host`) are checked by `cargo xtask architecture` against
[`xtask/architecture-policy.toml`](../../xtask/architecture-policy.toml) ([ADR 0089](../adr/0089-user-plugin-contract.md),
[ADR 0091](../adr/0091-user-plugin-code-runs-in-process-on-wasmi.md)).
A row applies as soon as its package exists, every list is closed, and a new dependency or
exemption is a reviewed edit of the policy with a reason.

- **I1, dependency closure.** From `cargo metadata` with `--features nocterm/user-plugins`, normal and
  build dependencies, the transitive `nocterm-*` closure of a crate must equal its row and the
  external closure must lie in its named list (for gpui crates, that list plus the closure of the
  vendored gpui).
- **I2, no std.** `nocterm-plugin-model`, `nocterm-plugin-api` and `nocterm-plugin-abi` are `#![no_std]` with
  `extern crate alloc`; `std` appears only as `#[cfg(test)] extern crate std;`.
- **I2b, no I/O paths.** In std-based isolated crates, resolved `std::fs`, `net`, `process`, `env`,
  `os` and stdin paths, and I/O dependency crates, are rejected through the same import, alias and
  re-export resolver as the domain ratchet. Allowance is zero; a module exception needs a reason.
- **I3, gpui receivers.** An identifier equal to a method of `App`, `Window`, `Context`, `AsyncApp`
  or `AsyncWindowContext` (inherent methods and methods of the traits implemented for them) in an
  isolated gpui crate fails unless it is listed with a reason.
- **I4, structural bans.** No `build.rs`, `include!` family, `#[path]` module, `macro_rules!`, proc
  macro other than serde and thiserror derives, `extern crate` other than the two forms above,
  `unsafe`, inline or global assembly, symbol attributes (`no_mangle`, `export_name`, `link_name`,
  `link_section`), or serde attribute taking a path string.
- **I5, providers.** The dependents of the provider port are a closed list.
- **Compile-out.** With default features the resolved closure of `nocterm` holds no user-plugin
  crate.
- **Guest-only crates.** A crate listed under `guest_only` (the plugin SDK, whose `export!` macro
  writes the symbols and manifest section of a guest module) has no row, because I4 would reject
  that macro. Instead it must be absent from the `nocterm` closure, with and without
  `user-plugins`; development dependencies do not count.
- **Test hooks.** A crate that offers a test-only constructor behind a `test-support` feature lists
  it under `test_features` in the policy (empty today). The feature must not be enabled by any
  normal or build dependency edge of `nocterm` with `user-plugins` (the default build holds no such
  crate); development dependencies may enable it.

These rules protect against host-side regressions. They do not cover I/O reached through an
allowlisted dependency's own API, nor a confused deputy; those are design-review items.

## History and maintained documentation

CI runs [`check-architecture-history.py`](../../scripts/check-architecture-history.py)
against each incoming PR-head commit. Pushes to `dev` and `master` also check the
actual event's before/head range, including GitHub-created merge or squash commits.
The integrated-history result is required by the aggregate `CI passed` job. This
post-integration check detects an invalid landing after it occurs; repository
operators must still use the prescribed merge method. The synthetic PR merge is
not the integration input. Once an inventory first appears, subsequent commits
in the same PR cannot raise or delete it. Commits changing `vendor/` must be
separate from commits changing `crates/` or `src/`. Existing mixed commits reachable
from the audited snapshot `084626ce53248982a5c086a9b854f89d8ff7da2b` are frozen
legacy debt. The source constant cannot change or disappear in incoming commits;
there is no command-line or environment override. Missing snapshot history fails
with a fetch diagnostic. New ordinary and merge commits remain subject to the rule.
A PR containing both areas
uses rebase merge, preserving the separate commits; the same rule applies to
`dev` → `master` promotion. Squash and merge commits would combine their files
and fail the per-commit policy. Required checks apply to the final PR head.
Locally, run the script with
`--base <recorded-target>`; setting `NOCTERM_ARCHITECTURE_BASE` makes the architecture
command run the same history check.

Maintained Markdown under `docs/` rejects session authorization and progress
phrases. Inline references to known repository types check their members;
fully qualified repository symbols must exist. Fenced examples and external
APIs are not treated as repository declarations. Accepted ADRs and
`docs/history/` retain their historical context. Generated references still have
the separate `cargo xtask docs --check` freshness gate.

Cargo `test-support` activation is restricted to development dependencies. Normal
and build dependencies cannot enable it directly, through feature aliases or through
default features. Other package features cannot forward to their own `test-support`.
An owning `test-support` definition may be empty or enable crate-local switches and dev-only
dependency support; its name does not exempt normal/build dependency edges, including
renamed bindings, weak optional forwarding or child default/feature aliases.
This restriction is based on Cargo metadata, independent of source filenames.
Workspace child feature/default aliases are traversed through `--no-deps`
metadata. Explicit external `test-support` activation is guarded; arbitrary
third-party alias definitions are outside this inspection scope. Support source still
undergoes the inventory checks; its `#[cfg(test)]` tests are excluded normally.
Existing future aliases returning `String` do not create new explicit error types
at each implementation: their defining API remains measured migration debt.
