# Nocterm native parser error layout observation

Base: published serde_json 1.0.151, MIT OR Apache-2.0. The recorded published
archive and upstream revision are in vendor/patches/sources.json.

Adds Error::allocation_bytes(), exposing the size of the existing private
ErrorImpl body without changing parsing, constructors, error categories, source,
positions, feature defaults, or allocation behavior. This value excludes nested
message strings and nested I/O errors; consumers must account for those separate
paths before using stock constructors. The bounded history Unicode-unit adapter
uses the actual stock parser and retains its original concrete Error.

Error::message_allocation_bytes() observes the exact current private Message
Box<str> length or 0, without formatting, cloning, copying or changing constructors.
Controlled typed-route callers use it only after prepaying source-derived parser/
formatting growth peaks, so an escaped original error retains its actual body and
message allocation instead of a temporary prospective peak envelope.
