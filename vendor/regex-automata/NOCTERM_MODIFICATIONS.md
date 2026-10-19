# Nocterm modifications

This directory contains regex-automata 0.4.18 from its published crates.io archive.
Its original MIT OR Apache-2.0 license files and publisher provenance are retained.

The opt-in `nfa::thompson::pikevm::managed` API supplies one explicit, non-cloneable
search scratch owner. A caller budget admits each real state/capture vector
layout before allocation. Epsilon-stack growth admits the actual old/new backing
overlap; refusal of optional growth headroom retries the exact needed capacity.
Typed budget refusals and `TryReserveError` allocator causes are preserved.
Cache/capture storage is destroyed before its allocation receipts.

The managed API and existing unmanaged API use the same PikeVM search and epsilon
transition implementation. The unmanaged path keeps its original vector growth
strategy. No new regex grammar, Unicode rule, match ordering, empty-match behavior,
compiled rule pool or global cache is introduced. Managed search aborts on a real
reservation failure, clears partial capture output, and supports retrying the
same cache after the caller's authority permits it.

An internal sparse-set constructor consumes already allocated dense/sparse tables.
The new public API does not expose mutable cache/reset operations that could grow
unaccounted backing.
