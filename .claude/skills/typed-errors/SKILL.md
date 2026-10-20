---
name: typed-errors
description: Define or extend thiserror error types for a fallible API that crosses a crate boundary, preserving the original concrete cause and converting to text only at display. Use whenever a new public fallible function, port or handle is added.
user-invocable: false
---

# Typed errors

Rule: AGENTS.md design rule 4; ADR 0046 (shared concrete causes). The
architecture check counts production `Result<_, String>` per package and the
count may only shrink.

## Procedure

1. **One error type per boundary**, in the crate's `error.rs` module, exported
   from `lib.rs` (see `crates/containers/src/error.rs`).
2. **Variants name what failed for the caller**, not where in the code:
   `MissingId { entity, index }`, not `ParseError2`. Carry the data the caller
   needs to react or display (indices, identifiers), not formatted text.
3. **Keep the original cause** with `#[source]` or `#[from]`. Never
   `map_err(|e| e.to_string())` and never store a formatted message in place of
   a cause.
4. **Shared or cloned causes** use `nocterm_core::error::SharedError<E>` (typed)
   or `SharedCause` (erased with `downcast_ref`). It keeps the original error as
   the next source frame without copying text.
5. **Refusals are variants**: stale authority, closed queues, unavailable
   capabilities are distinct variants, not a generic `Failed`.
6. **Text only at display**: views format an error with `Display` or a
   presentation function; logs use the source chain. Converting to `String` is
   allowed in the view, never in a port or service signature.
7. **Domain errors stay pure**: no I/O error types from adapters in domain
   signatures unless the domain contract defines them (e.g. session's `ExecError`).

## Tests

- One test per variant the caller can observe, asserting the variant (with
  `matches!`), not the message text.
- A test that `std::error::Error::source()` returns the original cause when one
  is retained.

## Review checklist

- [ ] no new `Result<_, String>` across the crate boundary
- [ ] each fallible public API names its error type
- [ ] causes retained with `#[source]`/`#[from]`/`SharedError`
- [ ] variants carry data, not formatted text

End with the Result block.
