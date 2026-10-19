# 0049. Panel coordination retains private membership and finite presentation work

## Context

The controls and view lifetimes described by [ADR 0045](0045-private-panel-view-owners.md)
were separate owners, while panel coordination still retained flat membership,
subscriptions, sign-in inputs and redraw state across sixteen implementation files.
Index-based selection needed adjustment when archived chats were prepended or removed.
A detached redraw task outlived the state that requested it.

## Decision

AgentPanel coordinates twelve private collaborators across four implementation
files: panel, lifecycle, chat and composer. ChatList owns slots and selected original
EntityId. Each ChatSlot retains its Thread entity and that entity's observer subscription.
Prepend and admitted removal preserve selection by identity. The coordinator captures
each candidate document through the existing Runtime admission boundary before committing
membership removal; a refused admission keeps its slot and unsent input.

PanelNotices owns display errors, the ordered approval fingerprint and one coalesced
33-ms redraw. Request, installation and completion compare the same opaque ticket.
The owner retains the actual task, so dropping it cancels pending presentation work;
an old completion cannot retire a replacement request. PanelBindings retains the
existing supplied-settings, workspace and runtime installation subscriptions.

SignInCards owns native secret controls and their Enter subscriptions. Each immutable
submission retains the original Thread, Item, prompt token, input and prompt capability.
A new token creates an empty control even when its label is unchanged. Submission
consumes only its captured input, then validates its original authority; secrets never
enter durable chat or notice data.

Stateless shell, composer, queue, configuration, attachment, approval and transcript
renderers receive actual controls and bounded immutable projections. Their event
listeners invoke named actions carrying original entity, queue, advertisement or prompt
identities. Rendering does not expose mutable parent state or acquire runtime services.
Existing compatibility runtime/authentication lookup stays in coordinator seams.

Finite preview and image operations own result work and original list/composer/session
receipts. ImagePreparation owns a typed result future; that work captures the injected
inspector for native bytes/files without a mutable panel callback. Task installation
and accepted delivery remain with MessageList or ComposerView. Picker and queued-image
completion also validate original revision and prompt identity before applying effects.

## Consequences

Selection and observation lifetimes have one owner, and queued redraw work cannot
outlive its presentation owner. The existing five view owners keep their original
controls and state boundaries. Thread coordination, reconnect receipts, document
admission, HistoryReader permits, shared persistence and shutdown ordering remain
unchanged. This decision extends ADR 0045; it introduces no new entity, service,
protocol, data format, native I/O capability, global or shutdown handler. Runtime and
authentication installation remain separate work.
