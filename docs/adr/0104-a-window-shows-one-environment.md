# 0104. A window shows one environment

Status: accepted

Supersedes [0099](0099-windows-have-no-environments.md) entirely. It also supersedes the
decision of [0103](0103-account-data-is-sealed-on-the-device.md) that one chosen server keeps the
vault and the profiles of the whole application (`SelectedVault`), and the decision of
[0101](0101-profiles-synchronize-through-a-replica-beside-the-local-document.md) that the replica
sits beside the local document. The ciphertext rules of 0102 and 0103 stay in force.

## Context

A single list of connections and a single vault for the whole application, switched by choosing
one account server, mixed the data of this computer and of a hub on disk and on screen. The
user's model is simpler: a window shows this computer, or the account of one hub, and nothing
of the other.

## Decision

**A window shows exactly one environment** (`Environment { Local, Hub(HubId) }`,
`nocterm-hubs-model`): this computer, or the account of one hub. The connections, the vault and
the credentials of a terminal of the window come from its environment. Workspaces, containers,
files, monitor, snippets, the agent and plugins stay local features of every window; they work
over the connections the window opened.

**The account of a hub is a mirror in memory.** The application builds one `HubSource` per hub
on first use: a connection store without writers on disk, a `ProfileReplication` that
replicates it with the hub's replica, a `VaultService` over the account's vault, and the
terminal capability with that vault's `CredentialProvider`. Every window of that hub shares it.
Leaving the hub (signed out, revoked, removed) empties the mirror and locks the account's
vault. The replication state is in memory too: a new run starts from an empty mirror and
agrees with the hub again. The local window has its own installation without replication.

**A window is composed from its sources.** `WindowSources` (connections, terminal, vault,
settings pages, environment) is what `build_workspace` registers. A hub window registers its
vault with `register_scoped`, so `UnlockVault` dispatched inside it opens the dialog of that
vault and the workspace takes the focus after the window is composed.

**No mixing, read-only when not in step.** The list of a hub window is editable only while the
mirror is `Synced` or `Syncing`; a locked, offline or signed-out account shows a strip above
the list and nothing can be created or edited, because an edit that cannot reach the hub would
be lost with the memory. A closed vault is `ReplicaError::Locked` and `ReplicationStatus::Locked`
(distinct from unreachable), and asks for the unlock.

**The environment is chosen in the title bar.** A menu right after Help lists "Local", the
known hubs and "Sign in to server…". `EnvironmentOpener` (a port in `nocterm-hubs-ui`, named for
the menu's need, implemented by the application) switches this window or opens another.

## Consequences

- `SelectedVault`, `HubAccountSync`, the `[hubs] account` setting, `Departure` and the account
  server choice on the Servers page are gone. An existing `account` key is ignored.
- Nothing of a hub account is written to the local connection document; profiles that an earlier
  build copied there stay as ordinary local profiles.
- Offline start shows an empty list with "Server unreachable" until the hub answers; an
  encrypted offline cache is a separate decision.
