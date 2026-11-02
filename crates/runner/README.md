# Independent runner control plane

`nocterm-runner` is a headless executable. It owns a private persistent identity,
material store, encrypted native journal and runner session, and answers real
authenticated fresh-head probes using that journal. Without an `[execution]`
section it is a pure control plane: model and terminal execution are disabled and
it neither consumes nor acknowledges execution commands. With one, `run` also
composes the execution runtime (see below).

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

The response reports `execution_ready=true` only while the execution runtime is
serving commands with a valid boot, and `false` for a control-plane-only daemon
or after execution stopped. `init`, `enrol` and `refresh` run no execution and
always print `execution_ready=false`. The private local socket
accepts only health reads. There is no network administration endpoint. Manual
`refresh` is available only when the service is stopped and holds the same
exclusive private owners. A refresh with an unknown remote outcome fails closed;
it does not repeat the HTTP mutation or silently restore earlier credentials.

## Execution

Execution is enabled by two explicit, complete sections: `[execution]` (native
containment limits, the contained workspace, administrator-selected provider
endpoints with model allowlists and finite per-call cost ceilings) and
`[session_targets]` (named local/SSH targets and the default). Either one alone
is refused. The file never carries an API key: provider credentials reach the
runner only as sealed original commands from the account's devices and live in
bounded zeroizing memory. Hub enrolment of an execution runner must run `init`
with both sections present, because that creates the private command inbox.

`run` then, before reporting healthy: requires native containment (a missing
namespace or limit refuses startup, never falls back to uncontained execution),
replays the durable chat catalog, announces this process's one boot key to the hub,
and serves the authenticated command inbox. A separate task delivers the durable
outbox to the hub once a second. The boot key is valid for 30 minutes; at expiry
the daemon exits with a failure so the supervisor starts a fresh boot, because
devices can no longer deliver provider credentials to an expired one. Execution
requires Linux; elsewhere a configured runner refuses to start.

Fixed finite caps (not configurable): 8 concurrent runs (2 per account), 128 queued
runs, 64 staged provider/SSH answers, 128 live sessions (64 for the account), 16
per chat. Session metadata, snapshot, cleanup and command-ingress work each use a
separate bounded pool.

## Container image

A managed pool starts the runner from a container image that the server's
compose file names in `NOCTERM_RUNNER_IMAGE`, pinned by digest.
`deploy/runner/Dockerfile` builds it: a release `nocterm-runner` from this
workspace (the Rust version of `rust-toolchain.toml`) on a slim Debian with
`bubblewrap`, `util-linux` (`prlimit`), `bash` and CA certificates. It runs as
user `10001` and its entrypoint is the runner, so the host supplies the
arguments (`managed --config <path> --control stdin`).

```sh
docker build -f deploy/runner/Dockerfile -t registry.example.org/nocterm-runner:<tag> .
docker push registry.example.org/nocterm-runner:<tag>
docker inspect --format '{{index .RepoDigests 0}}' registry.example.org/nocterm-runner:<tag>
```

`docker inspect` prints `registry.example.org/nocterm-runner@sha256:<digest>`
once the image has been pushed; that value is the `NOCTERM_RUNNER_IMAGE` pin.
Rebuild and re-pin it when the runner changes. The containment runs `bwrap`
inside the container, so the container runtime must allow unprivileged user
namespaces for it.
