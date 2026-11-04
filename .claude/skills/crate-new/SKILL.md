---
name: crate-new
description: Add a workspace crate when placement establishes a distinct responsibility, with ownership card, allowed dependencies, typed ports and behavior coverage.
argument-hint: "<responsibility in one sentence> [target: master|dev]"
---

# New crate

Task: $ARGUMENTS

Follow AGENTS.md's Workflow and Definition of done. Record responsibility,
branch/base, layer, public surface, dependencies, risks and check scope in a
compact plan; [structured handoffs](../feature/plan-bus.md) are optional.

1. `placement` must justify a new crate with its criteria; otherwise use a module
   and `crate-extend`. Use `port-design`/`typed-errors` for collaborators and
   fallible APIs, `wiring` for installation, and `docs-adr` for the boundary decision.
2. Add `crates/<name>/Cargo.toml` like `crates/containers/Cargo.toml`: inherited
   edition/license/lints, `publish = false`, description and
   `[package.metadata.nocterm] layer = "<layer>"`. Add its path dependency under
   root `[workspace.dependencies]`; `crates/*` supplies workspace membership.
3. Start `lib.rs` with the ownership card (Owns, Does not own, Ports, Settings,
   Globals, Invariants). Name modules after responsibilities. Add typed errors
   for fallible APIs and feature-gated test-support fakes where needed.
4. Implement behavior with `behavior-tests` and explicit dependencies. Apply
   scoped `gate` checks after coherent edits; regenerate docs once dependency
   changes settle. Include dependency/notice checks for the new manifest.
5. Review with `design-review`, fix findings and finish the applicable input-based
   completion checks. Report responsibility, boundary changes, evidence and
   limitations. Commit/push/PR only when requested.
