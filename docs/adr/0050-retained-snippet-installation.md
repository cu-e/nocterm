# 0050. Snippet views retain their original installation and private draft controls

## Context

Snippet registration and editor saves acquired one global library. Installing a
second library could redirect an existing editor. Dialog background styling also
acquired settings through a compatibility global, while the editor mixed native
draft controls, attachment navigation and modal completion state.

## Decision

The snippets feature returns cloneable Installed with two private collaborators:
its exact library entity and supplied SettingsHandle. Bootstrap retains a successful
installation and passes it to each workspace and NewSnippet action. Panels, editors
and delete confirmations retain that same installation. It exposes immutable library
queries, named checked save/delete commands and observation; no global lookup or
mutable model callback selects authority. This follows the retained composition
boundary in [ADR0043](0043-retained-connections-installation.md).

SnippetDraft owns the original expected record, current binding/language facts,
actual name/description/code controls and visible validation error. Candidate
construction trims only name; literal description/code and exact profile/group
scope survive section navigation. Language selection updates the native highlighter.
AttachmentPicker owns actual search input, folded group identities and section.
The editor retains save receipt, subscriptions and original modal authority.

Save validates before admission and retains original expectation on failure.
Successful receipt advances that expectation; closing requires the captured original
dialog focus still contains focus. A newer stacked dialog survives completion and
the resurfaced saved editor remains editable. Dialog background receives explicit
TerminalSettings through Installed and uses existing TerminalStyle paint semantics.

Shared Persisted/ShutdownCoordinator ownership from [ADR0013](0013-persistence-shutdown-service.md)
continues to retain accepted writes through view/window release. Shutdown registration
failure is reported by bootstrap and skips feature registration; no replacement
unregistered library is exposed. [ADR0007](0007-bootstrap.md) remains the common
application/test composition boundary.

## Consequences

All windows share the installation provided by bootstrap; deliberately distinct
installations remain independent. Native snippet file loading, identity generation
and the existing viewless feature model remain migration debt for later adapter/
service extraction. This decision introduces no new crate, I/O capability, worker,
file format, settings schema, global or shutdown deadline.
