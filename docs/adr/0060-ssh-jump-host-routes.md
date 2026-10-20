# 0060. SSH jump-host routes are resolved from saved profiles before opening and established by the SSH adapter

Status: accepted

## Context

Servers behind a bastion could not be reached: a saved connection opened one
direct (or proxied) TCP connection. OpenSSH solves this with `ProxyJump`, where
each hop is a full SSH connection that forwards to the next. Profiles reference
other profiles by identifier, so a route can name a connection that is later
renamed, edited or deleted, and an unsaved edit must be able to stand in for
its stored version. Prompts for host keys and passwords must say which hop asks,
and a password saved for one host must not be offered to another.

## Decision

- A route is data of the profile (`Profile.jump_hosts`, ordered), not a session
  option. Each entry is either a saved connection or a literal
  `user@host[:port]`. An empty list is omitted from the serialized profile, so
  profiles without jump hosts keep byte-identical documents. Downgrading drops
  the field like any other field unknown to an older version
  ([ADR 0036](0036-connection-profile-native-boundary.md)).
- `nocterm-connections-model::jump_route::resolve_jump_route` is a pure function
  over the loaded profiles. It runs when a session is opened, expands saved
  entries transitively into an ordered list of resolved hops, uses the unsaved
  edit in place of the stored profile, detects cycles with a stack and fails
  with typed errors for a deleted connection, a cycle, or a route longer than
  `MAX_JUMP_HOSTS` (8).
- Only resolved values cross crate boundaries. `SessionSpec.jumps` holds
  `ResolvedJump` (a `JumpHost` of target and authentication, plus the saved
  credential that answers its secret prompts); `ConnectRequest.jumps` holds the
  `JumpHost` values. Profile identifiers and the model's entry type stay in the
  connections feature.
- The SSH adapter establishes the route by `direct-tcpip` chaining: connect and
  authenticate the first hop, open a forward through it to the second, run the
  SSH handshake over that stream, and so on to the destination. The configured
  proxy tunnel applies to the first connection only; later connections ride
  inside the previous one. Closing the session closes the chain from the
  destination outward.
- Each hop has its own host-key check against the known-hosts store and its own
  authentication, completed before a forward is requested through it. Prompts
  carry `PromptHop { index, target }`; the terminal labels them with the hop's
  address and shows no label for the destination.
- The terminal answers a hop prompt with a saved credential only when the
  credential belongs to that hop's target. Remembering an answer is refused for
  hop prompts, so a typed hop password is never written to a profile.
- A session is identified by its destination and its route (`HostAddress`: the
  target and the jump-host targets in order). Matching an open session by
  target alone, as the transfer queue's Retry and "open on host" do, would
  reach another machine whenever two tabs have equal targets behind different
  bastions, so those lookups, transfer jobs and program hosts compare the route
  too. A program tab (container log or shell) keeps the route of the session it
  runs on, and labels naming a routed host append `via <first hop>`.
- A remembered password is attached by `CredentialScope`. A session reached
  through jump hosts binds only to the profile it was opened from and that
  profile's recents; a direct session binds only to connections without jump
  hosts. A secret typed for one route is therefore never offered to the same
  address behind another, or around the route. A routed session that was not
  opened from a saved connection remembers nothing.
- The connection timeout applies to each hop separately. A failure is
  attributed to the hop that caused it, and a route over the limit is refused
  before any I/O.

## Alternatives

- Store the route in session options: options inherit global defaults and are
  not references to other profiles, so renames and deletions would leave
  stale copies.
- Resolve inside the SSH adapter: it would need profile and credential models,
  breaking the adapter's ignorance of profiles.
- Spawn the system `ssh -J`: loses in-process host-key and credential prompts
  and does not exist on every platform.

## Consequences

- An older version that rewrites the profile file drops `jump_hosts`
  ([ADR 0036](0036-connection-profile-native-boundary.md)); the profile then
  connects directly to the private address, and the host-key prompt is the
  only guard. Likewise, deleting a profile with jump hosts removes its
  recents instead of keeping them as plain destinations, so a recent cannot
  reach the target around the route with the profile's saved credential;
  recents of profiles without jump hosts still survive deletion.
- Changing only a profile's jump hosts discards the operating system and
  country detected for it, as changing its target does.
- Renaming or editing a bastion applies to every connection that references it
  at the next open; deleting it blocks those connections with a message that
  names the missing hop, until the route is edited.
- A route costs one SSH handshake per hop; hops are not shared between
  sessions.
- The editor shows the route, offers saved connections and addresses, and
  reports an unresolvable route (deleted connection, cycle, too long) as a
  validation error when saving.
