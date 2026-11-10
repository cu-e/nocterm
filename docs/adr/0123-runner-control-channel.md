# 0123. The managed runner reads chat tokens from a control channel

Status: accepted

## Context

ADR 0121 starts a managed runner with one boot document on standard input that
carries a single lease token. A chat that starts after the runner, or a token
that ends while other chats keep running, cannot reach it that way. The token
must stay out of files, logs, the hub and process arguments. The host (a private
repository) writes the bytes and this runner reads them, so the format is fixed
here and in the contract (`nocterm_hub_model::agents::ChatAccessFrame`, contract
ADR 0003). ADR 0121 is not edited; this decision extends it.

## Decision

**Framed standard input.** `nocterm-runner managed --config <path> --control stdin`
reads standard input as a stream of frames that stays open for the runner's life.
Any other value after `--control` is refused, and `--control` belongs to `managed`
only. Without `--control`, boot v1 is read to EOF exactly as ADR 0121 describes, the
runner reports `execution_ready=false` and no control channel exists.

**Frame 1: boot v2.**

```text
u32 BE  length n of the JSON body, 1 <= n <= 65536
[u8; n] UTF-8 JSON, one object
```

The object is boot v1 without `provider.token` and with `provider.pricing`;
`version` is 2 and unknown fields are refused (a `provider.token` among them):

```json
{ "version": 2, "instance", "agent", "lease", "name", "instructions",
  "provider": { "kind": "openai_chat" | "anthropic_messages", "base_url",
                "models": [..],
                "pricing": { "maximum_request_bytes", "maximum_output_tokens",
                             "maximum_call_cost_micros" } },
  "secret_names": [..], "allowed_hosts": [..],
  "ssh_targets": [ { "host", "user", "port" } ] }
```

`base_url` is validated as a broker endpoint as in ADR 0121. `pricing` is one set of
conservative ceilings (all greater than zero) applied to every listed model; the
host renders it from operator configuration and the broker stays the budget
authority. A frame over 64 KiB, a zero length, a short read or invalid JSON fails
start-up before any worker exists.

**Frames 2..: chat access.** Each later frame is the contract `ChatAccessFrame`,
byte for byte:

```text
u32 BE  body length, at most 252 (a frame is at most 256 bytes with this prefix)
u8      version = 1
u8      op: 1 = issue, 2 = revoke
[u8;16] chat id
issue only: u16 BE token length (1..=128), token text `nvt1_` + base64url
```

A revoke carries no token. The reader is `ChatAccessDecoder`: it refuses an
oversize length before buffering, an unknown version or op, a malformed token, a
truncated frame and bytes after the last field.

**Semantics.**

- ISSUE gives the chat a token for the broker: the runner builds a memory-only
  provider lease from it, replaces and revokes the chat's earlier lease, and wakes
  the chat's runs that wait for provider access. A repeated ISSUE is harmless.
- REVOKE forgets the token and revokes its lease; the chat's next model call fails
  with `NeedsProviderAccess`. A REVOKE of an unknown chat is harmless.
- A frame error (including oversize, unknown version, trailing bytes) or an I/O
  error closes the channel: every token is revoked, no later ISSUE is accepted and
  nothing is resynchronised. The runner keeps running; its chats fail with
  `NeedsProviderAccess`. The typed error is written once to standard error.
- EOF, even in the middle of a frame, means the lease ended: every token is
  revoked and the runner shuts down through the same path as SIGTERM.
- Tokens exist only in zeroizing memory. `Debug` of the token store and of a token
  never shows one. A lease built from a token lapses after the 30-minute cap of
  `ProviderLease`; the runner rebuilds it from the held token when a chat asks for
  one, until REVOKE or close. The broker alone decides when a token stops working.

**The reader.** A dedicated blocking thread reads an unbuffered duplicate of
fd 0 (never the standard library's buffered `Stdin`, whose buffer is not wiped) in
chunks of at most 512 bytes (zeroized), so the async runtime never waits on it and a slow writer
cannot block shutdown. Tokens reach the coordinator's runs through a `ProviderWake`
that calls `RunCoordinator::wake_provider`; the coordinator gains no field.

**Alternatives rejected.**

- An inherited descriptor 3: Docker cannot pass one to a container, while the
  bwrap pipe and the Docker attached stdin (`OpenStdin`, `StdinOnce` false) both
  carry standard input.
- Tokens in boot v2 or in files: only the frame channel serves the first chat and
  later chats the same way, and nothing touches a disk.

## Consequences

- The host writes boot v2 first, then a frame per `ChatAccess` ISSUE or REVOKE, and
  closes the pipe to end the lease; nothing else may be written to the pipe.
- Chats start and end while a runner keeps running, without a restart.
- A future frame layout is a new version byte in the contract; this reader refuses
  what it does not know.
- Containment and the broker credential source that use these tokens are ADR 0124.
