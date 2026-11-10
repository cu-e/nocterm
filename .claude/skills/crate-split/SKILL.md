---
name: crate-split
description: Move a responsibility between crates, checking layer dependencies, narrow contracts, characterization coverage and an ADR for the new boundary.
argument-hint: "<package>[::<module>] [target: master|dev]"
---

# Split a crate

Subject: $ARGUMENTS

Follow AGENTS.md's Workflow and Definition of done. Record branch/base, ownership
move, metric, risks and checks in a compact plan; use [structured handoffs](../feature/plan-bus.md)
when helpful. A crate-boundary change requires an ADR.

1. Survey ownership and references with `responsibility-map`; apply `placement`
   to each piece and check its dependency layer. A module spanning incompatible
   layers needs to be separated, not placed in the nearest crate.
2. Resolve boundary problems within the authorized design: split domain and
   presentation responsibilities, move a narrow contract down, or keep the
   responsibility in its existing crate. Present options to the user only when
   a material choice remains or the solution changes authorized scope/authority.
3. Use `port-design` for cross-boundary collaborators and reject dependency cycles.
   Design target crate/module, public surface, typed errors, wiring, move order,
   tests and the `docs-adr` decision. Use the `crate-new` skeleton if needed.
4. Characterize through the old API, move in coherent compiling steps and keep
   tests with the code they cover. Avoid compatibility shims unless the design
   requires one and gives its removal condition.
5. Update ownership cards, layer notes and ADR; regenerate references when the
   combined dependency changes settle. Use `gate` for both sides and consumers.
6. Review with `design-review`, fix findings and finish the applicable completion
   checks. Report ownership/metric changes, checks/results and limitations.
   Vendor and app changes require separate commits; commit/push/PR only on request.
