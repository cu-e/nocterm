# 0162. Windows holds directory handles for strict local filesystem capabilities

Status: accepted.
- Amends [0110](0110-local-history-environment-partitions.md): Windows now supplies the held
  directory capabilities that new v3 chat namespaces require.

## Context

New chats are environment-scoped v3 documents ([0110](0110-local-history-environment-partitions.md)).
Their partitions require held directory capabilities, so that one account's or project's
history cannot be redirected into another through a swapped directory or link.

`nocterm-localfs` implemented held capabilities only on Unix, with directory descriptors and
`*at` calls. Windows used the portable fallback, whose identities are locations rather than
held directories. Every save of a new chat on Windows was refused with
`EnvironmentUnsupported`, the queue paused, and the notice advised checking disk space.

## Decision

- On Windows a strict `DirectoryCapability` holds an open handle to every directory from the
  volume root down to itself. Each handle is opened with list access, without
  `FILE_SHARE_DELETE` and with `FILE_FLAG_OPEN_REPARSE_POINT`. While the capability lives,
  no held directory can be renamed, removed or replaced by a junction, so path operations
  below it resolve through the same directories that were checked.
- Every component is opened as itself; a reparse point (symbolic link, junction, mount
  point) or a non-directory is refused as `LinkRefused`.
- The identity is the volume serial number and file index read from the held handle, and
  is reported as native, so `is_held` is true because the handles are actually held.
- A child capability is opened below its parent's held chain. A child of a browse
  capability, which followed links and holds nothing, first re-establishes a strict chain.
- Staged publication, sources, listings and removal keep their existing Windows code. They
  run below a held chain, and publication still replaces the target atomically.
- Other non-Unix targets keep the portable fallback and the typed refusal of 0110.
- A failed save names its class: unsupported storage, a full volume, refused access or
  another failure. Only a full volume suggests freeing disk space.

## Consequences

- New chats save on Windows in every environment and context, with the same partition and
  owner checks as on Unix.
- While a capability is held, other programs cannot rename or delete the held directories;
  capabilities are short-lived, so this lasts for one operation.
- A directory another program holds open with delete access cannot be opened strictly until
  that program closes it.
- On ReFS the 64-bit file index is the lower part of the 128-bit file ID; identities are only
  compared within one operation.
