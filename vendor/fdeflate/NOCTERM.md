# Nocterm native decoder admission

Base: published fdeflate 0.3.7, upstream
c365c7e6ffa81feb2e1fb762eed7299f05c9b0ca, MIT OR Apache-2.0.

The original decompressor engine accepts an additive generic allocation budget
(default `Untracked`). Existing constructor/type spellings and legacy bounded
output APIs remain available. Managed constructors admit the two actual Huffman
box backings before construction. Secondary Huffman vectors and managed bounded
outputs admit replacement backings before allocation, physically free the old
backing, then retire its receipt. Inline receipt headers allocate no storage.

The explicit account is instance-local; no TLS or global allocator is installed.
Refusal remains observable by the operation driver, including when an optional
format metadata parser intentionally ignores an ordinary parsing error. The
native format owner must drop transferred output backings before their receipts.
This is an allocation ownership mechanism, not proof of an application's total
resident limit. No compression algorithm, checksum or accepted deflate grammar is
replaced or restricted.
