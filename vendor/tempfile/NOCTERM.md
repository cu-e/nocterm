# Nocterm admitted temporary-path addition

Base: published tempfile 3.27.0,
upstream 5c8fa12eb584931b4f1bccfde87eb72fbfa7dc61,
MIT OR Apache-2.0.

Nocterm adds `TempPath::from_prepared_path(Box<Path>)`: a safe ownership move
matching the original internal named-file TempPath constructor, with no copying,
current-directory query, normalization or allocation. It preserves the exact
caller-prepared spelling, including Windows drive-relative paths. Existing
constructors, creation, publication, cleanup and permission policies are unchanged.
The caller must retain allocation credit and prepare the operation path first.

Behavior tests extend the original namedtempfile suite, reuse its current-directory
lock/restore owner, and exercise original-box identity and real file cleanup.
Windows spelling tests require actual Windows execution; cross compilation alone
is not native runtime or allocator evidence.

Controlled native callers may query `path_error_allocation_bytes()` and move an
already admitted native path into `error_with_owned_path(io::Error, PathBuf)`.
Both original IoResultExt wrappers and the new function use the SAME private
PathError type, preserving its Display, Debug, error kind and source forwarding.
The helper does not clone its path. Callers separately admit the std io::Error
custom body and actual path backing; the helper is not an allocator proof.

The additive `anonymous::AnonymousOperations` port supplies per-instance owners at
allocation sites of the existing anonymous Unix engine. Legacy public APIs and
controlled callers execute the same TMPFILE decision, original fallback and
shared creation/retry helper. Native flags, random draws/reseed, retry count,
0600 creation and ignored unlink-result policy remain unchanged. Controlled name
storage is admitted before its constructor and retains its credit inseparably;
current-directory, joins, native conversions and original exhaustion errors use
narrow supplied operations. `exhausted_with_owned_path` invokes the original
error constructors and private PathError. Callers must prepay all their nested
standard-library allocations; the collaborator is not a global allocator meter.
