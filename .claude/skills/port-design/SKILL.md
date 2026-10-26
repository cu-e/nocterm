---
name: port-design
description: Design a narrow port (trait or handle) between a consumer and a capability — named for the consumer's need, with a typed error, a shared fake and injection in bootstrap. Use when code needs I/O, another feature, or a service it must not depend on directly. Not for contributions shown by the window shell, plugin settings or capabilities exposed to user plugins (plugin-api).
user-invocable: false
---

# Port design

Input: a consumer and what it needs ("the snippets panel needs to paste into the
user's terminal"). Output: a "Ports" section in `30-design.md`
(`.claude/skills/feature/plan-bus.md`), then code.

Rules: AGENTS.md "Where code goes" item 5, design rules 3 and 4; ADR 0006
(workspace ports), 0017 (local filesystem capabilities), 0032 (narrow terminal
ports), 0003 (credential store port).

## 1. Reuse first

Search for an existing contract before writing one:

- `nocterm-session`: `Transport`, `RemoteFs`, `LocalFs`/`LocalMutations`,
  `CredentialStore`, `HostExec`, `ProgramOpener`.
- `nocterm-workspace`: the narrow terminal and connection seams
  (`LocalTerminal`, `UserTerminalPaste`, `TerminalCapabilities`,
  `ConnectionDirectory`, …).
- `nocterm-store`: `Persisted`, `SnapshotWriter`, `AppendPersisted`, `ShutdownCoordinator`.

Reuse a contract only if the consumer needs **all** of it. Never add a method
to an existing broad trait for a new feature pair: write a separate port.

## 2. Shape

- **Name for the consumer's need**: `UserTerminalPaste`, not `TerminalApi`;
  `ConnectionDirectory`, not `ConnectionsService`.
- **Minimal surface**: the methods this consumer calls today. More than about
  six methods, or methods only another consumer uses, means two ports.
- **Owner of the contract**: the lowest layer both sides may depend on —
  a domain crate (pure contract, e.g. session), `workspace` for feature-to-feature
  seams, a service crate for GPUI handles. Use `placement` if unsure.
- **Authority**: if the capability acts for a specific session, prompt or epoch,
  the port takes the captured identity/receipt and refuses stale ones
  (ADR 0032; retained installation in ADR 0042, 0043, 0050–0053).
- **Errors**: one `thiserror` type per port boundary (`typed-errors`); no
  `String` errors and no protocol- or SDK-specific types in the signature
  (ADR 0058).
- **No globals**: the consumer receives the port through its constructor or
  installed dependencies (`wiring`), never through `cx.global()`.

## 3. Fake and implementations

- The fake lives with the contract behind `test-support` (session fakes are in
  `crates/session/src/testing/`: `InMemoryFs`, `FakeExec`, `ScriptedTransport`,
  `FakeCredentialStore`, `Gate`, …). Extend an existing fake before adding one;
  if no shared home fits, propose one instead of a crate-local duplicate.
- `test-support` is enabled only from `[dev-dependencies]` (the architecture
  check rejects anything else).
- The real implementation lives in the adapter (I/O) or the providing feature.
- Composition injects it in `src/bootstrap.rs` (`wiring`).

## Output (in `30-design.md`)

```
### Port <Name>
- Consumer: <crate/type> needs <need>
- Contract crate/layer: <crate> (<layer>) — <why lowest common layer>
- Methods: <signatures>   Error: <Type> (variants)
- Authority captured: <session/prompt/epoch or none>
- Fake: <existing fake extended | new fake in <crate> testing>
- Implementation: <crate>; injected in bootstrap at <step>
- Rejected alternative: <broader trait / direct dependency> because …
```

End with the Result block.
