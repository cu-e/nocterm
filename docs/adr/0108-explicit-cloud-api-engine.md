# 0108. Cloud API engines expose completed turns to the host

Status: accepted

Partially supersedes [0105](0105-provider-usage-through-acp-only.md) for the explicit
Cloud API engine only. Local ACP agents retain that decision's authentication,
subscription credential and usage boundaries.

## Context

A persistent Cloud run needs provider streaming without a desktop agent process.
An SDK tool callback cannot decide that an effect is authorized or durably recorded.
A partial tool argument stream and a serializable SDK history are insufficient
proof of either provider completion or effect completion.

## Decision

`nocterm-agent-engine` is an adapter implementing cloud-model's `AgentEngineFactory`
and `AgentEngine`. The factory derives its immutable account scope, provider API
root and model allowlist from the original finite provider lease. It reads no environment, home directory,
subscription token, SSH key or plugin registry. OpenAI-compatible Chat Completions
and Anthropic Messages use Rig's actual provider wires at a full Git revision.

The adapter drives Rig's sans-I/O `AgentRun`; it installs no SDK tool executor.
It advertises the existing eight terminal catalog schemas, validates the assembled
arguments through that same catalog and returns calls to the host. It waits for
Rig's actual provider stream to finish and validates the terminal reason before
exposing a tool batch. Unknown, malformed and incomplete calls cannot authorize
an effect. The host retains grant checks, intent/dispatch commits, tool execution
and outcome reconciliation.

Host step/index slots correlate tool results independently of provider call IDs
or names. A private versioned, bounded checkpoint retains only derived SDK state
and binds it to the original account, runner, chat, run, epochs, configured
provider/model, neutral prior-history seed and budget. The host anchors its outer checkpoint to the exact
authoritative journal revision; it must validate that receipt before restoration.
An interrupted, dropped or failed advance refuses further advancement/checkpointing;
recovery uses the host's preceding durable checkpoint.

The HTTP transport accepts only the configured POST destination. It disables
redirects and proxy discovery, requires HTTPS for production and resolves DNS
through a public-address filter used by the connecting client. The real API key
is absent from SDK configuration and checkpoints; the transport inserts it only
after checking the destination. Library HTTP header and TLS buffers still contain
the credential while a request is live; a trusted runner administrator can read
its process memory. Test-only loopback HTTP is unavailable in production builds.

Text and thought progress use bounded neutral records and awaited delivery.
Provider metadata cannot create trusted tool presentation or execution facts.
New user prompts receive bounded neutral history from the authoritative journal,
with validated complete intent/result pairs and stable host-derived wire IDs.
They cannot resume an unrelated run's checkpoint. This first adapter explicitly
requests reasoning off: a model requiring thinking, or a response requiring opaque
or signed thinking continuation, is refused as an unsupported capability before
any tool batch. It does not silently discard required provider continuation state.
Reported usage retains unknown values. Catalog-estimated costs are not published
as provider charges. Before each HTTP request the adapter reserves its remaining
output-token cap, reconciling only reported usage; absent usage consumes that
reservation. This is a local bound, not an exact provider bill or a cancellation
refund. The host installs an explicitly bound memory-only provider lease, lasting at
most thirty minutes. Its shared zeroizing key cannot be retargeted to another
account, endpoint or model allowlist by cloning the handle. Explicit revocation
and monotonic expiry clear the key and abort every active engine advance,
including one blocked by progress delivery. Losing that lease is an explicit
`NeedsProviderAccess`; restarting a worker requires fresh provider access. The
host owns concurrent account admission.
Native streaming must be polled on the host's Tokio runtime.

## Alternatives

SDK auto-execution hides the durable pre-dispatch barrier. A second hand-written
provider wire duplicates decoding and finish semantics. Ambient provider discovery
can consume credentials belonging to a local subscription agent. Trusting EOF,
provider call IDs, model metadata or a successful checkpoint as authorization
confuses observation with durable authority.

## Consequences

The engine can be tested against real HTTP fixtures without executing tools or
using real provider credentials. Each provider needs independent completion,
error, truncation, usage, cancellation and restoration coverage. Rig upgrades
require passing that same suite and intentionally changing the checkpoint format;
its checkpoint is never a second execution ledger. A server refusing a request
retains its HTTP failure status but its body is discarded to avoid logging an
API key or prompt echoed by a remote endpoint.
