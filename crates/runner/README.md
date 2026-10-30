# Independent runner control plane

`nocterm-runner` is a headless executable. Its current explicit mode is
`control-plane`: model and terminal execution are disabled. It owns a private
persistent identity, material store, encrypted native journal and runner session;
it answers real authenticated fresh-head probes using that journal. It neither
consumes nor acknowledges execution commands.

Create a private TOML configuration (mode `0600`) with a private absolute state
path. Obtain the hub instance ID, SHA-256 authority pin, account ID/account public
DH key and runner assignment from trusted setup. TLS uses the normal public CA
policy unless `tls_sha256` explicitly pins a certificate. Hex fields contain
exactly 32 bytes, except the 16-byte runner ID.

```toml
state = "/private/path/nocterm-runner"

[hub]
address = "https://hub.example.org:443"
instance = "trusted-instance-id"
authority = "<64 hex characters>"
# tls_sha256 = "<64 hex characters>"

[account]
id = "account-id"
public_key = "<64 hex characters>"

[runner]
id = "<32 hex characters>"
key_epoch = 1
```

Initialize once, then enroll with the one-use ticket on standard input, never in
arguments or the environment:

```sh
cargo run -p nocterm-runner -- init --config /private/path/runner.toml
cargo run -p nocterm-runner -- enrol --config /private/path/runner.toml
cargo run -p nocterm-runner -- show-public --config /private/path/runner.toml
cargo run -p nocterm-runner -- run --config /private/path/runner.toml
```

`show-public` emits a bounded TOML document containing the hub origin/authority,
account public-key fingerprint, runner assignment and its three public keys.
Compare these setup values through a trusted channel before registering them on
the account. The document contains no enrollment ticket, private key, credential
or state path. Export it before `run`: both commands require the same exclusive
identity owner, so export refuses while the daemon is running. Keep the private
configuration and session files private; they are not setup documents.

`init` refuses existing identity state. `run` restores the durable runner session,
services head probes and serializes credential refresh with those probes. Closing
a desktop does not own this process. Run it under a process supervisor with its
own private account principal. `SIGINT` and `SIGTERM` stop admission, retire the
source, join tasks and wait for actual guarded reads/bodies under one deadline.
A missed drain deadline produces failure.

A second process may request bounded read-only local health:

```sh
cargo run -p nocterm-runner -- health --config /private/path/runner.toml
```

The response always includes `execution_ready=false`. The private local socket
accepts only health reads. There is no network administration endpoint. Manual
`refresh` is available only when the service is stopped and holds the same
exclusive private owners. A refresh with an unknown remote outcome fails closed;
it does not repeat the HTTP mutation or silently restore earlier credentials.

This executable does not yet compose command execution, provider boot publication,
SSH approval exchange or the complete per-account process containment graph.
