# 0012. Architectural debt is an inventory that may only shrink

## Context

Dependency layers and source size limits do not constrain the responsibilities
inside a crate. A large stateful type can satisfy the file limit by distributing
its methods across files. An expectation that becomes unfulfilled catches a
resolved complexity exception, but does not prevent adding another expectation.
ADR 0008's claim that exceptions can only shrink therefore needs an additional
mechanical constraint.

## Decision

Keep exact current exceptions in `xtask/baseline.toml`. `cargo xtask architecture`
measures production structure and API shape through Cargo module trees and the
Rust syntax tree. It rejects new or increased entries; decreases require reducing
the inventory. Fully qualified declaration identities separate same-named types
and resolve imported names and type aliases.

Measure structures over 15 fields, public state fields in feature/service crates,
local types implemented across more than four files, GPUI globals and panicking
lookups, shutdown hooks, string error types, and existing domain I/O dependencies
and references. Plain data exemptions are individual named types with explicit
reasons. Complexity expectations include tests and require the existing reason
`predates the limit`, including inside conditional attributes.

CI compares inventory entries across incoming PR-head commits, including the commit
that first introduces the inventory. Integration pushes to dev/master additionally
check the actual before/head history, so host-created merge or squash commits are
examined after landing. The aggregate CI passed job includes that result; branch
protection should require that aggregate. The post-integration check detects a
policy violation after landing and does not replace the prescribed merge method. It also rejects a commit combining vendor and
application changes. Preexisting mixed commits reachable from the audited snapshot
`084626ce53248982a5c086a9b854f89d8ff7da2b` remain frozen legacy debt; the checked
source constant cannot be changed or removed and has no runtime override. Missing
snapshot history fails closed. Every newer ordinary or merge commit follows the
separation rule. A PR containing correlated vendor-only and application-only
commits is rebase-merged, including integration-to-release promotion, so those
commits remain separate. Squashing or a merge commit combining both areas is
forbidden. Required checks apply to the final reviewed head.
Maintained documentation rejects session-status prose and
checks unambiguous repository symbol references.

No automated inventory regeneration or aggregate exception budget is provided.
A responsibility extraction removes or reduces its owner's entries; a rename or
relocation is not permission to introduce new exceptions. A later decision must
specify any validated transfer of an existing exception between owners.

## Consequences

- Existing debt remains buildable while each improvement is measurable.
- Adding fields, globals, broad implementation spread or string errors cannot
  be hidden by changing file boundaries or increasing the inventory.
- ADR 0008's source/function limits remain in force; the inventory strengthens
  its no-growth policy.
- Source inspection cannot prove responsibilities or expand arbitrary macros.
  Design review remains mandatory for those boundaries and named data exemptions.
- Inventory metrics and documentation limits are described in
  [the guardrails reference](../architecture/guardrails.md).
