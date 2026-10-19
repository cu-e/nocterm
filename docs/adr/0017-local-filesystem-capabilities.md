# 0017: Local filesystem capabilities belong behind session ports

## Context

Explorer combined presentation with native listing, completion, statistics and
link-safe mutation code. Downloads held a separate filesystem implementation.
The feature's transfer queue and completion bindings were GPUI globals, so a
window's dependencies were hidden and tests used real files or inspected view state.

Read-only navigation and mutation need different link semantics: a user may browse
through a directory link, while deletion, rename and source streaming must not let
an ancestor replacement redirect the operation. Native names may also be non-UTF8;
a displayed string cannot identify the file to mutate.

## Decision

`nocterm-session::local_fs` owns synchronous, typed contracts: `LocalFs` for
read-only browsing and held sources/directories, `LocalMutations` for mutations,
and `LocalDirectory`, `LocalSource`, `LocalCursor` and `LocalStage` for held work.
Paths and native names remain `PathBuf` and `OsString`. Pure cancellation, checked
bounds and statistics accounting belong to session; it performs no real I/O.

The `nocterm-localfs` adapter implements these contracts. Explorer dispatches
blocking calls on its background executor. Browsing may follow directory links;
strict traversal rejects linked ancestors and pins Unix directory handles.
Unix regular source opens check the same opened handle and use nonblocking open before
rejecting special files. Portable traversal retains its active-ancestor path race
limitation; Windows source opens reject reparse points on the opened handle.

Listings are bounded to 100,000 entries and 8 MiB display-name bytes. Walks and
removals are bounded to one million encountered entries and depth 256. Failed
metadata and skipped links consume the walk budget. Cancellation and limits retain
partial diagnostics; removal errors report already removed entries.

Staging belongs to the held-directory contract, rather than a configuration-file
serializer: streaming, no-clobber collisions and publication must retain the same
directory identity. A create collision preserves the candidate for another name.
Successful publication relinquishes temporary-name ownership immediately. Failure
before publication is distinct from a durability warning after publication; drop
cleans only the still-owned temporary candidate.

Explorer receives private `Deps` through `install`, which returns `Installed`.
Application composition passes the installed handles to every workspace and captures
them for New Window. Registration captures the transfer queue explicitly; bindings
are installed once. `FilesPanel` coordinates typed intents between independent
`LocalPane` and `RemotePane` entities. Public listing queries expose presentation
without granting mutation of owner state. Session's optional `test-support` feature
provides an in-memory implementation of these ports through dev-dependencies.

## Alternatives

Keeping native I/O in the feature preserves hidden platform coupling and duplicated
security mechanisms. Returning file descriptors or `std::fs::File` from the domain
would expose platform details and let consumers bypass bounds. Making all navigation
strict would remove existing read-only link browsing. A GPUI global would hide the
collaborators again and bind queue ownership to initialization order.

## Consequences

Features depend on domain contracts; only application composition chooses the native
adapter. Linux physical tests cover held identity, links, FIFO rejection, bounds and
publication cleanup; Windows and macOS library checks do not establish runtime race
guarantees. Explorer's opener processes and the legacy transfer backend remain
separate migration debt; this decision does not claim their I/O has already moved.
