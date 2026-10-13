# 0033: Native workspace and sandbox capabilities are injected

Status: accepted. Supersedes native workspace preparation and sandbox placement
portions of [0005](0005-agent-runtime-service.md),
[0010](0010-session-lifecycle.md) and [0011](0011-chat-configuration.md).

## Context

Runtime prepared and canonicalized workspaces directly. AI sandbox policy resolved
filesystem paths and environment variables and emitted native bubblewrap arguments.
Settings queried that native domain API. These operations mixed native capabilities
with policy and service lifecycle and made deterministic consumers depend on host
paths and installed binaries.

## Decision

AI defines WorkdirResolver and SandboxProbe contracts and typed errors with native
causes. Its SandboxPolicy validates supplied lexical and canonical path facts and
builds an ordered semantic mount plan. ACP prepares private directories through the
shared core helper, resolves native path facts, probes availability and translates
mounts into bubblewrap arguments. Runtime schedules its explicitly injected resolver.
Settings receives its probe through explicit dependencies retained by bootstrap for
every window. Features and services do not depend on the native adapter.

Preparation supplies one canonical root for process startup, session creation,
resume, fallback and Connected delivery. The existing candidate connection key,
serial, cancellation and retained orphan cleanup retain their lifecycle ownership.
Immediately before isolated launch ACP resolves paths again, rejects a changed
canonical root or protected credential alias, and checks executable availability
using the sanitized agent environment. UI availability is display evidence, never
launch authority. Missing optional roots remain absent; other native resolution
errors fail closed and retain their typed source. Isolation never falls back to a
plain process.

The mount plan preserves broad writable caches, hidden ancestors, the dedicated
workspace, hidden descendants and explicit read-only shared roots in that order.
Lexical and canonical credential containment are both checked, including symlinked
HOME and credential roots. Unsupported platforms retain ordinary nonisolated launch
and settings guidance.

## Alternatives

Moving only domain filesystem calls would leave native preparation in Runtime.
Caching UI availability as launch authority could allow a removed binary or a
different agent PATH to change isolation. Best-effort canonicalization after a
permission error would silently accept incomplete protection facts.

## Consequences

Policy and consumer tests use supplied immutable facts and shared scripted ports;
physical path, permission, process and namespace tests belong to ACP. No new
service, global, writer or quit lifecycle is introduced. Path validation remains
point-in-time validation, not protection against races through held directories.
