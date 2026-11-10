# 0121. Managed pools are a trusted gateway

Status: accepted

## Context

A pool host starts agent runners on behalf of an account. The runner must reach
a model provider without ever holding the real API key, and without the hub or
a pool operator being able to read personal definitions.

## Decision

The pool host is a trusted gateway for the credentials it brokers, and for
nothing else. It decrypts a sealed definition with its own private key, starts
`nocterm-runner managed --config <mount>/runner.toml` and writes one boot
document to the runner's standard input. The runner never receives the provider
key, only a virtual token that works against the host's credential broker, is
bound to one lease (chat = lease id) and can be revoked. The broker URL comes
from the host's `broker.public_url`. Usage counters are reported to the host.

`runner.toml` has the schema of the manual runner configuration (`state`
absolute, `[hub] address, instance, authority[, tls_sha256]`, `[account] id,
public_key`, `[runner] id, key_epoch = 1`) without `[execution]` and
`[session_targets]`. Boot document, JSON, `version: 1`, unknown fields refused,
at most 64 KiB:

```
{ "version": 1, "instance", "agent", "lease", "name", "instructions",
  "provider": { "kind": "openai_chat" | "anthropic_messages",
                "base_url", "models": [..], "token" },
  "secret_names": [..], "allowed_hosts": [..],
  "ssh_targets": [ { "host", "user", "port" } ] }
```

Secrets are names only. The runner validates `base_url` as a broker endpoint
(public https, or private and loopback hosts for a broker on the host's network)
and never writes the configuration's boot document to disk. The manual
`init`, `enrol` and `run` flow is unchanged.

The managed mode currently validates the document, reports a summary and runs the
control plane; composing contained tool execution with the broker credential
source is separate work, so `execution_ready=false` is reported.

Capacity is released with capability bit 64: publications carry `retry_until_ms`
and an explicit chat deletion is signed by the runner and sent with `RetireChat`
until the hub reports the stream compacted. A hub without the bit works as before.

## Consequences

- A compromised pool host exposes the credentials it brokers and the definitions
  sealed to it; it cannot read other pools' definitions or account history.
- A leaked virtual token is useless outside the broker and ends with its lease.
- Pool trust is pinned per hub on the desktop and a key change must be confirmed.
