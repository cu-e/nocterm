# 0036. Connection documents and native value acquisition use explicit ports

## Context

Saved profile and recent formats shared a feature module with UUID generation.
Editor defaults, timestamps and document loading/writing were acquired by the
foreground feature/bootstrap path, alongside its GPUI publication lifecycle.
This coupled pure collection rules and UI construction to native capabilities.

## Decision

`nocterm-connections-model` owns profiles, recents, detected-fact formats, checked
mutations and draft validation. Collection state has private fields and specific
transitions; callers cannot mutate arbitrary records through iterator escape
hatches. Identifier bytes, user/home defaults and timestamps are supplied values.
Profile parsing retains UUID format compatibility without native entropy.

`ConnectionRepository`, `ProfileIds` and `ConnectionClock` are narrow domain
ports. `nocterm-connections-io` implements files, native user/home discovery,
UUID entropy and Unix time. Repository failures preserve typed document/operation
context and native error sources. Malformed profiles retain a read-only diagnostic;
recents and detected facts retain their rebuildable fallback policy.

Application preparation loads documents/defaults on its existing background
executor before bootstrap. Bootstrap installs prepared inputs rather than opening
connection files or constructing network clients. The app bridges repository calls
into `nocterm-store::SnapshotWriter`; the existing shared worker executes writes
and retains checked publication/coupled shutdown ordering. Profiles continue the
foundation `save_preserving` configuration semantics; recents/facts use `save`.
No new persistence worker or atomic publication helper is introduced.

Editor opening, reload and quick-connect receive the existing connection entity
and its injected defaults/identifier source. A separate installed-handle abstraction
is unnecessary for this boundary. The memory-only composition uses deterministic
supplied values and inert observations without constructing native adapters.

## Alternatives rejected

- Moving the feature wholesale to a domain crate retains GPUI and I/O coupling.
- A new defaults/clock global preserves hidden lookup and lifecycle dependencies.
- A new file writer loop duplicates shared publication and shutdown ownership.
- Replacing UUID/TOML formats during extraction creates an unrelated migration.

## Consequences

Document formats, checked conflicts, recent ordering and credential matching stay
compatible. Unknown top-level profile data and retained comments keep the existing
writer semantics; unknown per-profile fields are not newly retained.

The feature still owns GPUI publication and remote OS observation scheduling with
its existing compatibility globals. This decision changes acquisition and pure
format ownership, not those lifecycle boundaries. Protocol-neutral OS observation
and explicit feature/service installation remain independent responsibilities.
