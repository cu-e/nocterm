# 0090. The plugin host brokers every effect, and providers only read what the UI already shows

Status: accepted

## Context

[ADR 0089](0089-user-plugin-contract.md) fixed what a user plugin can say and ask for. Something
has to turn that into behavior: decide what a plugin may see, apply the effects it asked for,
remember consent between runs, and keep a faulty or hostile package from loading the application.
Doing that in each feature crate would give every consumer its own idea of a grant, a rate limit
and a gesture, and would let a status bar widget start a sampler or a connection on a package's
behalf.

## Decision

**Ports and host.** `nocterm-plugin-ports` (service layer) holds the traits `Observe<C>` and
`Apply<C>` for one capability each, `Surface` for acting on a plugin's own contribution, and
`PluginProviders`, an immutable record of one window's providers built in the application's
composition. `nocterm-plugin-host` (service layer) holds the `PluginHost` entity. It is the only
caller of a provider. It checks the grant bound to `(PluginId, ContentHash)`, samples observations
no faster than 1 Hz (4 Hz for a panel with focus), queues the effects of an event (at most eight),
requires a pointer or an unrepeated Enter or Space for an effect that needs a gesture, rate limits
each capability, and checks the grant, enablement, gesture, rate and surface again when the effect
is applied. The contribution that raised the effect must still be registered and owned by the
plugin, and so must the contribution an effect acts on. Disabling, replacing, removing or faulting a plugin drops what is
queued for it, and three traps or missed deadlines in a session disable it. At most 16 plugins are
enabled and 32 renders happen per second.

**Passive providers.** A provider belongs to the crate whose state it reads and only answers from state the user's own UI
already holds:
`ShellCapabilities` in `nocterm-workspace` (`workspace.kind`), `MonitorUsage` in
`nocterm-monitor-ui` (`monitor.usage`), `TransferCounts` in `nocterm-files` (`transfers.counts`)
and `PluginNotices` in `nocterm-ui` (`notifications.own`, `notifications.post`). A provider reads
state that exists for the user's own UI and never creates, keeps alive or widens a sampler, watch,
subscription or connection; without that state it answers `unavailable`. The monitor reading is
published by the status bar widget when it creates its monitor, so a window without the widget
offers none. The monitor answers `unavailable` once its plugin is disabled, because the widget and
its monitor are released with it. A provider never reads an entity that may be updating when it is
asked: the workspace provider keeps the last state from the window's own events instead of
reading the shell on request. Providers sit behind the `user-plugins` feature of their crate; the isolation policy
lists the crates that may depend on the port (I5).

**Notices.** A plugin notice has a `NoticeKey` of its own class (plugin id and a local key), so it
can replace its earlier notice but never one of the application. A plugin keeps at most three
notices and all plugins together at most sixteen; neither class evicts or counts against the
application's notices, and a notice that asks for a host action is never shown as one.

**Records and data.** `PluginRecords` (which package, the consent, enablement and an audit ring) and
`PluginData` (setting values and the plugin's storage, at most 4 KiB and 64 KiB) are ports of the
host with their document types, codecs and bounds. The application implements them over the shared
persistence service: records as one snapshot document, data as one private file per plugin under
the state directory through the keyed driver, flushed at shutdown. A broken data file gives
defaults and is replaced only by the user's next change, and until then the plugin's own storage
effects are refused as `unavailable` without touching the file; a file larger than its cap or a link is
not read.

## Consequences

- A package can show what the user already sees and post bounded, rate limited notices; a window
  that has not created a widget or panel offers nothing from it.
- A new observation or effect needs a capability in the contract, a provider in the crate that owns
  the state, and an entry in the policy file; the host does not change.
- The host calls providers synchronously and defers the effects it applies. An observation
  provider must therefore answer without reading an entity that may be updating; an effect or
  surface provider runs only from the deferred apply and may read one.
- The application adapters for records and data are composed with the runtime that calls the host.
