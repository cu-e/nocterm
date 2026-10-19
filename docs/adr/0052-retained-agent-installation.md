# 0052. Agent views retain runtime, policy and host installation authority

## Context

Agent panels and threads discovered a runtime global, terminal-auth global and
AI policy through shared UI. Deferred operations could resolve another installation
at execution rather than retain the authority that accepted the request. Images,
layout, metadata and settings were already supplied independently.

## Decision

The service exposes cloneable RuntimeHandle over one private Runtime entity. Named
commands preserve typed results and existing synchronous ordered SessionEvent
routing. Immutable projections expose preferences, usage and document identity;
one take_history command drains saved chats and catalog atomically. Runtime owns
AI schema registration on its actual supplied SettingsHandle before registry reads.
No runtime Entity escape or arbitrary mutable update callback is provided.

The feature returns Installed over one immutable private Installation retaining
runtime, settings, metadata, image ports, layout, optional terminal-auth opener and
the settings subscription. Each ChatHost pairs that installation with its original
weak Workspace. Deferred preferences/usage, save/hydration and authentication retain
that exact runtime/opener; live policy checks read the same supplied settings at
the original operation stage. Authentication keeps its existing Send+Sync opener
and original terminal installation/window authority.

Installation owns the existing settings subscription, whose callback keeps a weak
runtime. Root retains Installed and supplies it to every workspace. Per-window
subscriptions retain original window handles and weak views. Shared shutdown capture
keeps RuntimeHandle, never Installed, Workspace or coordinator, and returns existing
App-free work. Runtime still emits policy cancellation and admission events in its
established order. No per-thread master policy observer is added.

Initialization returns Installed and a separate typed shutdown registration status.
Registration refusal is reported while preserving the existing usable feature;
it does not trigger a fallback runtime or silently omit agent views. Standalone
settings-page fixtures register their AI section explicitly; shared UI has no AI
policy dependency or compatibility lookup.

## Consequences

Separate installations remain independent and surviving/new windows retain the same
runtime and host capabilities. Panel/Thread ownership, process cleanup, persistence,
AI policy semantics and native authentication transport remain unchanged. Metadata
allocation still occurs once through the supplied source. This complements
[ADR0005](0005-agent-runtime-service.md), [ADR0013](0013-persistence-shutdown-service.md)
and [ADR0040](0040-native-chat-metadata.md). ACP signatures and version-one physical
history remain in use; neutral history and native environment policy are separate
boundaries. No worker, I/O capability, settings schema or quit handler is introduced.
