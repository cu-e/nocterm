# 0008. Function size and complexity limits shrink a grandfathered list

## Context

Several functions exceeded hundreds of lines and high cognitive complexity,
and nothing stopped new ones.

## Decision

Clippy limits functions to 80 lines and cognitive complexity 20, and files stay
at or under 800 lines. Functions that predate the limits carry
`#[expect(clippy::too_many_lines, reason = "predates the limit")]`; splitting
one makes the expectation unfulfilled and fails clippy until the attribute is
removed. `#[allow]` for these lints is rejected by `cargo xtask architecture`.

## Consequences

- The list of exceptions can only shrink, with no extra tooling.
- `xtask/oversized-files.toml` is empty and may not grow.
