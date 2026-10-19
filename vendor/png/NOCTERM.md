# Nocterm explicit native decoder admission

Base: published png 0.18.1, upstream
2a3f980245e3ae38b82ade96533e7b450e8477bb, MIT OR Apache-2.0.

The original Decoder, Reader and StreamingDecoder engines accept an additive
allocation-account type parameter defaulting to fdeflate::budget::Untracked.
Original constructor and type spellings remain available. Managed constructors
admit the actual chunk, inflater, metadata, text, unfiltering, row scratch and
transform closure backings before construction or replacement. Receipts follow
physical payload fields and do not allocate their own headers. Metadata uses the
original Info storage; there is no parallel decoded graph or replacement parser.

An optional corrupt ICC profile remains ignored as in the original parser. The
managed operation driver must separately check the instance account's first
sticky allocation refusal before returning image facts, since an optional
metadata parser may intentionally ignore its ordinary format error. The account
must reject subsequent admission after such a refusal. Existing format grammar,
checksum options, animation policy and legacy decoder limits remain unchanged.

These hooks cover decoder paths, not encoder/thumbnail allocations or downstream
rendering/cache lifetimes. Application-wide physical allocator proof remains a
separate obligation. Native owners must retain each receipt until its associated
physical backing has actually been freed; a receipt is not a memory estimate.
