# 0113. A native journal has an authenticated account catalog

Status: accepted

Refines [0109](0109-native-cloud-journal-and-effect-recovery.md): SQL routing
columns do not establish the complete set of chats whose effects need fencing.

## Context

An authenticated chat header names its active effects. That protects an effect
only after the chat is found. Selecting chats through an unauthenticated account
column lets a changed index hide the entire chat before its header is checked.
Treating a missing catalog as an empty account would reproduce the same failure.

## Decision

Each physical native journal belongs to one explicitly configured account and
runner identity/key epoch. `JournalCatalogBinding` carries the known
`AccountScope`, `RunnerId` and `RunnerKeyEpoch`; these are supplied by composition,
never discovered from database selectors or inferred from a host address. A claim
for another binding fails closed. Account/runner rotation requires an explicit
future migration and cannot create another partition for the same chat.

`JournalCatalogCipher` is a mandatory injected domain port for account/runner
catalog authenticated encryption. Its context binds that complete identity and a
positive catalog revision. It has a purpose separate from chat records, commands
and grants, and requires neither a retained account root key nor a chat key.
The native adapter supplies no plaintext production implementation or implicit
key discovery. Its implementation must compare the full native binding, including
the account public-key fingerprint, with configured verified account/runner pins
before mapping public contract coordinates. Fields absent from those coordinates
must not lose that expected-pin check. Coordinate construction proves no account
or key ownership. Native serialization is versioned Postcard and is not a signed hub
format. Plain serialization/decryption buffers are zeroized after use.

A singleton encrypted catalog lists every exact chat stream, including tombstones.
The common account/runner binding is stored once and unique bounded chat IDs
reconstruct the exact `StreamBinding` values. Adding the first head of a chat
updates that catalog in the same SQLite transaction. Ordinary writer renewal does
not invent a new catalog entry. Catalog plaintext has a 2 MiB hard cap, ciphertext
a separate bounded authentication allowance, and configuration may lower its
byte/chat ceilings. At most 100,000 chat IDs may be decoded, without trusting an
unbounded serialized sequence allocation hint. SQLite's database quota includes
the catalog.

Creation and opening are separate native operations. `create_new` uses exclusive
private file creation and durably commits an authenticated empty catalog before
returning. `open_existing` requires the expected schema and an authenticated
catalog. Missing data, an old unreleased format or interrupted initialization
requires explicit recovery; no automatic catalog/index rebuilding occurs.

Target-effect lookup starts from the known authenticated catalog, never a SQL
account/deleted/state filter. It checks the complete physical chat row set against
the catalog, checks all routing coordinates and authenticates every required
header before accepting its active effect list. Unknown extra or missing physical
rows are errors. Target/principal/workspace values remain encrypted; no unkeyed
search digest of those values is introduced.

An open driver retains the highest observed catalog revision and its ciphertext
digest as a refusal fence against an older or changed root during that owner's
lifetime. This is not a second authoritative ledger or an external durability
anchor. After restart, a coherent historical authenticated catalog together with
matching historical heads and data cannot be distinguished from current state
without a separately trusted head. This decision does not claim protection against
such whole-state rollback or replacement of the runner's trusted private storage.
The trusted ancestry and SQLite VFS limitations of 0109 remain.

## Consequences

Account workers need separate private physical journal directories and explicit
create/open composition. Opening does not depend on the key of an arbitrary first
chat, so deleting a chat cannot remove the catalog's crypto anchor. Unresolved
effects and tombstones continue to occupy catalog entries until an explicit future
retention/migration policy can preserve their fencing guarantees. Complete row
checks are bounded by deployment quotas; large catalog deployments need measured
capacity/latency limits rather than an unbounded discovery scan.
