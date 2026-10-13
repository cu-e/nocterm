# 0039. Remote OS observations use a checked domain identity and injected adapter

## Context

Connection facts combined remote stream acquisition and release parsing with
GPUI lifecycle and the OS glyph catalog. Early read failures skipped explicit
close; optional probing erased transport and decoding failures.

## Decision

`nocterm-connections-model` owns checked `OsId`, supported identity data,
release aliases, platform-marker eligibility and Windows home classification.
`Known::set_os` accepts checked identity while legacy stored strings retain their
existing serde shape. Glyphs, labels, colors and SVG rendering stay in the feature.

`RemoteOsDetector` accepts the held session `RemoteFs` capability and returns a
transport-independent typed result. Application preparation injects the native
adapter through the existing private observation dependencies; inert composition
makes no remote requests. No additional globals, service lookup or quit hooks
are introduced.

`nocterm-connections-io` reads `/etc/os-release` then `/usr/lib/os-release`, bounds
release bodies to 16 KiB and checks the requested chunk bound before extending
its buffer. Metadata identifies nonregular paths and final links without
interpreting legacy error strings. Absence and unsupported operations permit
fallback; unknown failures retain their native source. A successful alternative
observation can recover from an earlier optional failure. If no observation
succeeds, the first meaningful failure is returned.

Every opened reader is explicitly closed after successful or failed acquisition.
Close failure is a typed best-effort diagnostic and does not replace the primary
result. Cancellation releases the reader through Drop; it does not claim to run
an asynchronous close after the future has been dropped.

## Alternatives rejected

- Moving the glyph catalog into the domain retains presentation responsibility.
- Inspecting arbitrary `FsError::Other` text to classify symlink refusal ties
  policy to protocol-specific wording and hides unknown failures.
- A feature-to-adapter dependency couples views to native acquisition.

## Consequences

Existing OS priority, aliases, stored strings and presentation are preserved.
Probe-once policy and manual icon precedence stay in the feature. Existing fact
publication races and compatibility globals remain separate lifecycle migration
work; this boundary adds no new authority or race guards.
