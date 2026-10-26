# 0072. Directory selection shares read-only rules, adapters and controls

Status: accepted

## Context

Explorer navigation and workspace creation both need literal path entry,
directory completion and Home, Up and Refresh actions. Reimplementing Explorer's
input in a feature would duplicate keyboard, cancellation and path semantics.
Features cannot depend on each other.

## Decision

`nocterm-explorer` owns pure local and remote path resolution and completion
rules, plus narrow read-only directory contracts. Native local paths preserve
their identity; remote paths use POSIX semantics. Resolution does not lexically
remove symlink-parent traversal or interpret shell syntax.

`nocterm-directory-io` adapts the existing supplied local and remote filesystem
capabilities to those contracts. It owns neither a connection nor another native
traversal implementation. A missing remote filesystem never selects a local
filesystem instead. Home discovery uses the original source's capability.

`nocterm-ui` owns the shared path input, completion popup and navigation controls.
Explorer acknowledges navigation through its existing browser loader. A directory
picker can acknowledge a selection through the read-only directory contract.
Workspace creation and Explorer use the same editing and completion owners.

Every asynchronous result belongs to its original source and request generation.
Editing, source replacement and navigation invalidate older results. Failed
navigation retains the draft. Successful reading confirms access only for that
operation; it grants no mutation capability. A picker cannot validate container
paths through its host's filesystem.

## Alternatives

- Copying the input into workspace creation duplicates behavior and race fixes.
- Importing the Files feature creates a feature dependency for a shared control.
- Passing broad filesystem mutation capabilities to the picker exceeds its
  consumer's needs.
- Listing paths through shell commands adds quoting and output parsing to a
  filesystem capability already provided by the application.

## Consequences

Both consumers share path behavior and keyboard tests. Files retains full browser,
transfer and mutation ownership. Application composition supplies the same native
filesystem implementations through a read-only projection; the component does not
discover application services or choose another host.
