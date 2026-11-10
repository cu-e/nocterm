# 0100. The hub is a route step: the first connection of an SSH route can be a relay

Status: accepted

## Context

A connection reaches a host through a proxy or through jump hosts (ADR 0060). A Nocterm hub is
reachable from places a user's computer is not, and it can open TCP connections from its own
network or from one of its nodes. Offering it as a step of the route lets a profile reach hosts
behind it without a VPN, while the SSH session itself stays the user's: its host key, its
`known_hosts` and its sign-in.

The first connection of a route is a socket today. A hub offers a stream of bytes over its own
protocol instead, and that protocol is the adapter's secret (ADR 0096). `nocterm-session` is
domain code with no I/O and no hub types, and `nocterm-ssh` may not depend on `nocterm-hub-io`.
The two adapters also run on separate Tokio runtimes.

## Decision

**The route names the hub as text.** `ProxyConfig::Hub { address, node }` is one more proxy
kind. `address` is the string the user typed; `node` optionally names one node of that hub that
dials from inside its network. Only the address and node are checked in the session (not empty,
no whitespace, bounded). What they mean is the hub adapter's.

**The SSH adapter asks a port for a stream.** `nocterm-session` defines `RelayDialer`:
`dial(hub, node, target)` returns a `RelayStream`, or a typed `RelayError`
(`NotSignedIn`, `Refused { reason }`, `NodeUnavailable`, `Unreachable`, `Closed`; refusals are
`RelayRefusal::{Policy, PrivateNetwork, Port, Limit}`). `SshTransport::with_relay` supplies the
dialer; without one a route through a hub fails to connect instead of falling back. The SSH
handshake then runs over the stream exactly as over a socket: the host key checked is the
destination's, authentication is the destination's, and a hub that is a hop never sees inside
the connection. Only the first connection of a route may be a relay; jump hosts are reached by
`direct-tcpip` forwards on it as before.

**A stream is two bounded channels.** `RelayStream` holds two `async-channel` channels, one per
direction, each of `RELAY_FRAMES` (16) frames of at most `RELAY_FRAME_BYTES` (32 KiB). It is
created only by `RelayStream::pair()`, which fixes both bounds in one place and gives the relay
its end as a `RelayPeer`. Nothing else crosses between the two runtimes, so neither needs the
other's reactor, and a side that stops reading stalls the other side's sends instead of growing
a buffer. The SSH adapter bridges its end to a `tokio::io::duplex` pipe of two frames. The
hub adapter bridges its end to the hub's stream, which is bounded the same way and ends in
HTTP/2 flow control. No unbounded buffer exists between the SSH handshake and the hub's socket.

**Closing and cancelling work from both sides.**

- Dropping the transport's sender is end of file for the target, after the frames already
  queued; the target may still answer. The target closing cleanly is end of file for the
  transport, which may still send.
- Dropping the transport's receiver, both ends, or the session ends both directions: the hub
  adapter stops waiting for the target (even a silent one) and releases the hub's stream, which
  aborts it.
- A failure of the hub's stream ends both directions; the transport sees its channels close.
- Dropping the future of `dial` before it completes abandons the request. A stream already
  handed out lives on its own task until its channels close or the hub adapter stops.

**`nocterm-hub-io` implements the port.** `HubRelay` shares the runtime and the signed-in
clients of `HubLinks` and is composed in `src/startup.rs`, where the SSH transport receives it.
It finds the hub by address among the hubs this device is signed in to: the address is read the
way the `[hubs]` section reads one (scheme, case and the default port do not matter) and
looked up among the pooled clients, never contacted or signed in on demand. A hub the device is
not signed in to is `NotSignedIn`. The contract names a refusal by a code and keeps its text
for display, so the code decides and the text is never read: `RELAY_DENIED` is
`Refused { Policy }`, `RELAY_LIMIT` is `Refused { Limit }`, `NODE_UNAVAILABLE` with a named
node is `NodeUnavailable`. `PrivateNetwork` and `Port` are in the port for a hub that says so;
this contract does not distinguish them. Any other failure is `Unreachable`, and the original
failure stays the `source()` of the error.

If the hub adapter cannot start, no relay is supplied and such profiles fail to connect with
that reason; everything else is unaffected.

## Alternatives

A transport of its own for hub routes would repeat the SSH handshake, host-key policy and
authentication, or leave them out. Treating the hub as an SSH jump host would need the hub to
speak SSH. A `Stream` trait object returned by the port would let an adapter hand back an
unbounded buffer, and it would force the two adapters to agree on a runtime. Passing hub types
into `nocterm-session` would break the layer rule and ADR 0096. Having `nocterm-ssh` depend on
`nocterm-hub-io` would give the SSH adapter the hub protocol.

## Consequences

- The hub learns the destination address, when a connection opened and how much it carried,
  never what is inside it (ADR 0096 and the server's own records).
- A route through a hub works only while the device is signed in to that hub; the SSH session
  does not trigger a sign-in.
- `RelayRefusal::PrivateNetwork` and `Port` stay unused until the contract names them.
- The bounded queues cap the memory one relay holds at a few hundred KiB per direction.
